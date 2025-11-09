#![no_std]
#![no_main]

use battery_tester_common::{
	AllowUndercurrent, BIReply, BiCommand, COMMAND_MAX_SIZE, ClearFault, Fault, FaultKind,
	I2CError, LoadState, Measurement, MilliAmp, MilliVolt, REPLY_MAX_SIZE, Reset,
};
use defmt::{error, info};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_nrf::{
	Peri, bind_interrupts,
	gpio::{Input, Pull},
	peripherals::{self, P0_04, P0_14, P0_26, P1_00, TWISPI1},
	pwm::SimplePwm,
	twim::{self, Frequency, Twim},
	uarte::{self, Uarte, UarteRx, UarteTx},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Instant, Ticker};
// use fixed::types::{I16F16, I18F14, U16F16};
use microbit_side_lib::{
	BAT_CONNECT_DEBOUNCE_MS, DaqDataQueue,
	ina260::{self, Averaging, BVConvTime, INA260Config, OperMode, Register, SCConvTime},
	pwm::{HeaterCmd, PwmCtrl},
	twim_err_to_common,
};
use panic_probe as _;
// use sht4x::Sht4xAsync;

static CMD_CH: Channel<CriticalSectionRawMutex, BiCommand, 4> = Channel::new();
static REPLY_CH: Channel<CriticalSectionRawMutex, BIReply, 4> = Channel::new();

pub type I2C = Twim<'static>;

/// adress is GND, GND (both pads not connected).
pub const INA260_VIN_ADDRESS: u8 = 0x40;

bind_interrupts!(struct Irqs {
	UARTE0 => uarte::InterruptHandler<peripherals::UARTE0>;
	TWISPI1 => twim::InterruptHandler<peripherals::TWISPI1>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
	info!("Starting...");

	let p = embassy_nrf::init(Default::default());

	let i2c_sda = p.P1_00;
	let i2c_scl = p.P0_26;
	let i2c_driver = p.TWISPI1;
	// RING2 - P0.04/P0_04 - P2
	let bat = p.P0_04;
	let btn_a = p.P0_14;
	let uarte = p.UARTE0;
	let rxd = p.P1_08;
	let txd = p.P0_06;

	//PWM
	let pwm = SimplePwm::new_1ch(p.PWM0, p.P1_02); // p1.02 = P16
	let pwm_ctrl = PwmCtrl::new(pwm);

	//UART
	let mut uart_conf = embassy_nrf::uarte::Config::default();
	uart_conf.parity = embassy_nrf::uarte::Parity::EXCLUDED;
	uart_conf.baudrate = embassy_nrf::uarte::Baudrate::BAUD230400;
	let serial = Uarte::new(uarte, rxd, txd, Irqs, uart_conf);
	let (serial_out, serial_in) = serial.split();

	spawner.spawn(serial_reply_task(serial_out)).unwrap();
	spawner
		.spawn(power_task(
			pwm_ctrl, i2c_driver, i2c_sda, i2c_scl, bat, btn_a,
		))
		.unwrap();
	spawner.spawn(serial_in_task(serial_in)).unwrap();
}

#[embassy_executor::task]
async fn serial_reply_task(mut serial_out: UarteTx<'static>) -> ! {
	info!("init serial reply task");
	assert!(REPLY_MAX_SIZE <= u8::MAX as usize);
	let mut out_buf: [u8; REPLY_MAX_SIZE] = [0; REPLY_MAX_SIZE];
	loop {
		let reply = REPLY_CH.receive().await;
		let out_msg = postcard::to_slice(&reply, &mut out_buf).unwrap();
		let out_len = out_msg.len() as u8;
		// info!("len: {}", out_len);
		if let Err(e) = serial_out.write(&[out_len]).await {
			error!("write len error: {}", e);
			continue;
		}
		if let Err(e) = serial_out.write(&out_msg).await {
			error!("write msg error: {}", e);
		}
	}
}

#[embassy_executor::task]
async fn serial_in_task(mut serial_in: UarteRx<'static>) -> ! {
	info!("init serial in task");
	assert!(COMMAND_MAX_SIZE <= u8::MAX as usize);
	let mut in_buf: [u8; COMMAND_MAX_SIZE] = [0; COMMAND_MAX_SIZE];
	let mut len_buf = [0u8; 1];
	loop {
		match serial_in.read(&mut len_buf).await {
			Ok(()) => {
				// get msg len
				let msg_len = len_buf[0] as usize;
				// get slice of msg len
				let in_msg = &mut in_buf[..msg_len];
				// read exact msg length
				match serial_in.read(in_msg).await {
					Ok(_) => {
						let cmd: BiCommand = postcard::from_bytes(in_msg).unwrap();
						CMD_CH.send(cmd).await;
						// info!("msg: {}:{:?}", msg_len, &in_msg);
					}
					Err(e) => {
						error!("read msg error: {}", e);
					}
				}
			}
			Err(e) => {
				error!("read len error: {}", e);
			}
		}
	}
}

#[embassy_executor::task]
async fn power_task(
	mut pwm_ctrl: PwmCtrl,
	i2c_driver: Peri<'static, TWISPI1>,
	sda: Peri<'static, P1_00>,
	scl: Peri<'static, P0_26>,
	bat: Peri<'static, P0_04>,
	btn_a: Peri<'static, P0_14>,
) -> ! {
	info!("Init power task");
	// TODO: pull down here makes a voltage divider with the SparkFun Opto-isolator Breakout?
	// it should be pull none because the OI circuit is connected to ground or vcc?
	let mut bat_present = Input::new(bat, Pull::None);
	let mut fault_clear_btn = Input::new(btn_a, Pull::None);
	let mut i2c_conf = twim::Config::default();
	i2c_conf.frequency = Frequency::K250;
	let mut i2c = Twim::new(i2c_driver, Irqs, sda, scl, i2c_conf, &mut []);

	info!("waiting for battery reconnect");
	wait_bat_reconnect(&mut bat_present, BAT_CONNECT_DEBOUNCE_MS).await;

	loop {
		i2c_init_loop(&mut i2c, &mut fault_clear_btn).await;
		let fkind = power_ctrl_loop(&mut i2c, &mut bat_present, &mut pwm_ctrl).await;
		pwm_ctrl.set_cmd(HeaterCmd::Off);
		let fault = Fault {
			kind: fkind,
			time: Instant::now().as_millis(),
		};
		info!("waiting for fault clear");
		wait_fault_clear(&mut fault_clear_btn, fault).await;
		info!("waiting for battery");
		wait_bat_present(&mut bat_present, BAT_CONNECT_DEBOUNCE_MS).await;
	}
}

async fn power_ctrl_loop(
	i2c: &mut I2C,
	bat_present: &mut Input<'static>,
	pwm_ctrl: &mut PwmCtrl,
) -> FaultKind {
	/// collect data @ 10Hz
	const DAQ_INTERVAL_MS: u64 = 100;
	/// Turn off heater if we don't get a command from the PC for this many ms
	const COM_TIMEOUT: u64 = 1_250;
	loop {
		let mut measurement: Option<Measurement> = None;
		// do this so the ticker doesn't store ticks while we wait for fault clear
		let mut com_timeout_ticker = Ticker::every(Duration::from_millis(COM_TIMEOUT));
		let mut allow_undercurrent = AllowUndercurrent::default();
		let mut daq_queue = DaqDataQueue::default();
		let mut daq_ticker = Ticker::every(Duration::from_millis(DAQ_INTERVAL_MS));
		loop {
			match select3(
				daq_ticker.next(),
				CMD_CH.receive(),
				com_timeout_ticker.next(),
			)
			.await
			{
				Either3::First(_daq_interval) => {
					match daq(
						i2c,
						&bat_present,
						pwm_ctrl,
						&mut daq_queue,
						allow_undercurrent,
					)
					.await
					{
						Ok(Some(new_measurement)) => {
							info!(
								"daq: {}, {}, t: {}, d: {}",
								new_measurement.vbat,
								new_measurement.ibat,
								new_measurement.dt,
								new_measurement.duration
							);
							let _old_measurement = measurement.replace(new_measurement);
						}
						Ok(None) => {}
						Err(fk) => return fk,
					}
				}
				Either3::Second(cmd) => {
					match cmd.load {
						LoadState::Off => {
							pwm_ctrl.set_cmd(HeaterCmd::Off);
						}
						LoadState::On => {
							pwm_ctrl.set_cmd(HeaterCmd::On);
						}
					};
					let reply = BIReply {
						// if there's a measurement, take and send it
						measurement: measurement.take(),
						fault: Ok(()),
					};
					REPLY_CH.send(reply).await;
					if let Reset::Yes = cmd.reset {
						pwm_ctrl.set_cmd(HeaterCmd::Off);
						break;
					}
					allow_undercurrent = cmd.allow_undercurrent;
					com_timeout_ticker.reset();
				}
				Either3::Third(_com_timeout) => {
					pwm_ctrl.set_cmd(HeaterCmd::Off);
					error!("lost comms");
				}
			};
		}
		info!("disconnect and reconnect battery");
		wait_bat_reconnect(bat_present, BAT_CONNECT_DEBOUNCE_MS).await;
	}
}

async fn daq(
	i2c: &mut I2C,
	bat_present: &Input<'static>,
	pwm_ctrl: &mut PwmCtrl,
	daq_queue: &mut DaqDataQueue,
	allow_undercurrent: AllowUndercurrent,
) -> Result<Option<Measurement>, FaultKind> {
	if bat_present.is_low() {
		error!("Battery disconnected");
		return Err(FaultKind::NoBattery);
	}

	// IBat
	let milliamps = ina260::get_amps(INA260_VIN_ADDRESS, i2c)
		.await
		.map_err(|e| FaultKind::I2C(I2CError::InaVinCurrent(twim_err_to_common(e))))
		.inspect_err(|f| error!("I2C read milliamps error:\n{}", f))?;

	if bat_present.is_low() {
		error!("Battery disconnected");
		return Err(FaultKind::NoBattery);
	}

	// VBat
	let millivolts = ina260::get_voltage(INA260_VIN_ADDRESS, i2c)
		.await
		.map_err(|e| FaultKind::I2C(I2CError::InaVinVoltage(twim_err_to_common(e))))
		.inspect_err(|f| error!("I2C read millivolts error:\n{}", f))?;

	// IBat in range/heater fault check
	pwm_ctrl.watchdog(millivolts, milliamps, allow_undercurrent)?;

	Ok(daq_queue
		.push(milliamps, millivolts)
		.map(daq_to_measurement))
}

async fn wait_fault_clear(btn_a: &mut Input<'static>, fault: Fault) {
	loop {
		loop {
			match select(CMD_CH.receive(), btn_a.wait_for_falling_edge()).await {
				Either::First(cmd) => {
					// send reply
					if let ClearFault::Yes = cmd.clear_fault {
						let reply = BIReply {
							measurement: None,
							fault: Ok(()),
						};
						REPLY_CH.send(reply).await;
						return;
					}
					let reply = BIReply {
						measurement: None,
						fault: Err(fault),
					};
					REPLY_CH.send(reply).await;
				}
				Either::Second(_btn_a_fell) => break,
			}
		}
		// debounce - wait for button to be down for 1 second (1000 ms)
		let mut ticker = Ticker::every(Duration::from_millis(1000));
		loop {
			// hold for 1 second (1000 ms)
			match select3(ticker.next(), btn_a.wait_for_high(), CMD_CH.receive()).await {
				Either3::First(_held_for_time) => {
					let reply = BIReply {
						measurement: None,
						fault: Ok(()),
					};
					REPLY_CH.send(reply).await;
					return;
				}
				Either3::Second(_released_too_soon) => break,
				Either3::Third(cmd) => {
					// send reply
					if let ClearFault::Yes = cmd.clear_fault {
						let reply = BIReply {
							measurement: None,
							fault: Ok(()),
						};
						REPLY_CH.send(reply).await;
						return;
					}
					let reply = BIReply {
						measurement: None,
						fault: Err(fault),
					};
					REPLY_CH.send(reply).await;
				}
			}
		}
	}
}

fn daq_to_measurement(pwr: (MilliVolt, MilliAmp, Instant, Duration)) -> Measurement {
	Measurement {
		vbat: pwr.0,
		ibat: pwr.1,
		dt: pwr.2.as_millis(),
		duration: pwr.3.as_millis(),
	}
}

async fn wait_bat_present(input: &mut Input<'static>, ms: u64) {
	loop {
		// wait for battery connection
		loop {
			match select(input.wait_for_high(), CMD_CH.receive()).await {
				Either::First(_battery_present) => break,
				Either::Second(_cmd) => {
					let reply = BIReply {
						measurement: None,
						fault: Ok(()),
					};
					REPLY_CH.send(reply).await;
				}
			}
		}

		// debounce - wait for battery to be connected for "ms" time
		let mut ticker = Ticker::every(Duration::from_millis(ms));
		loop {
			match select3(ticker.next(), input.wait_for_low(), CMD_CH.receive()).await {
				Either3::First(_timer_passed) => {
					// timer passed and the input never went from hi to low
					info!("battery connected");
					return;
				}
				Either3::Second(_battery_dc) => {
					// input went low (battery dc) before timer ended
					// wait for rising edge again
					break;
				}
				Either3::Third(_cmd) => {
					let reply = BIReply {
						measurement: None,
						fault: Ok(()),
					};
					REPLY_CH.send(reply).await;
				}
			}
		}
	}
}

/// Wait for the battery to connect and stay connected for ms - milliseconds
/// If the battery was already connected it must be disconneted and reconnected
async fn wait_bat_reconnect(input: &mut Input<'static>, ms: u64) {
	loop {
		// wait for initial battery connection
		loop {
			match select(input.wait_for_rising_edge(), CMD_CH.receive()).await {
				Either::First(_initial_contact) => break,
				Either::Second(_cmd) => {
					let reply = BIReply {
						measurement: None,
						fault: Ok(()),
					};
					REPLY_CH.send(reply).await;
				}
			}
		}

		// debounce - wait for battery to be connected for "ms" time
		let mut ticker = Ticker::every(Duration::from_millis(ms));
		loop {
			match select3(ticker.next(), input.wait_for_low(), CMD_CH.receive()).await {
				Either3::First(_timer_passed) => {
					// timer passed and the input never went from hi to low
					info!("battery connected");
					return;
				}
				Either3::Second(_battery_dc) => {
					// input went low (battery dc) before timer ended
					// wait for rising edge again
					break;
				}
				Either3::Third(_cmd) => {
					let reply = BIReply {
						measurement: None,
						fault: Ok(()),
					};
					REPLY_CH.send(reply).await;
				}
			}
		}
	}
}

async fn i2c_init_loop(i2c: &mut I2C, fault_clear_btn: &mut Input<'static>) {
	loop {
		match init_i2c(i2c).await {
			Ok(_) => break,
			Err(fault) => {
				error!("I2C init error:\n{}", fault);
				wait_fault_clear(fault_clear_btn, fault).await;
			}
		}
	}
}

async fn init_i2c(i2c: &mut I2C) -> Result<(), Fault> {
	// adress is GND, GND (both pads not connected).
	info!("init_i2c()");
	let mut conf = INA260Config::new();
	// 4 sample average * 4.156 ms conv time * 2 (both I & V) = 33.248 ms per measurement
	conf.set_averaging_mode(Averaging::AVG4)
		.set_operating_mode(OperMode::SCBVC)
		.set_sccov_time(SCConvTime::MS4_156)
		.set_bvcov_time(BVConvTime::MS4_156);

	info!("write ina configs");
	ina260::set_config(INA260_VIN_ADDRESS, i2c, conf)
		.await
		.map_err(|e| {
			let kind = FaultKind::I2C(I2CError::InaVinConfig(twim_err_to_common(e)));
			Fault {
				kind,
				time: Instant::now().as_millis(),
			}
		})?;

	let mut rd_buffer = [0u8; 2];
	i2c.write_read(
		INA260_VIN_ADDRESS,
		&[Register::DIE_ID.addr()],
		&mut rd_buffer,
	)
	.await
	.map_err(|e| {
		let kind = FaultKind::I2C(I2CError::InaVinId(twim_err_to_common(e)));
		Fault {
			kind,
			time: Instant::now().as_millis(),
		}
	})?;
	let id = u16::from_be_bytes(rd_buffer);
	let chip_id = id >> 4;
	let die_rev_id = id & 0b1111;

	info!(
		"setup VIN INA260... CHIP ID: {}, DIE REV: {}",
		chip_id, die_rev_id
	);
	Ok(())
}
