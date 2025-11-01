use core::prelude::v1::Err;

use battery_tester_common::{AllowUndercurrent, FaultKind};
// use battery_tester_common::HeaterCmd;
use defmt::{error, info};
use embassy_nrf::pwm::{Prescaler, SimplePwm};
use embassy_time::Instant;

use crate::{MilliAmp, MilliVolt};

pub struct PwmCtrl {
	cmd: HeaterCmd,
	pwm: SimplePwm<'static>,
	change_time: Instant,
}

impl PwmCtrl {
	pub fn new(mut pwm: SimplePwm<'static>) -> Self {
		init_pwm_out(&mut pwm);
		Self {
			cmd: HeaterCmd::default(),
			pwm,
			change_time: Instant::now(),
		}
	}

	/// sets pwm output based on desired heater state
	pub fn set_cmd(&mut self, new_cmd: HeaterCmd) {
		set_pwm(&mut self.pwm, new_cmd);
		match (self.cmd, new_cmd) {
			// if there was a change record the time
			(HeaterCmd::Off, HeaterCmd::On) | (HeaterCmd::On, HeaterCmd::Off) => {
				self.change_time = Instant::now();
			}
			_ => {}
		};
		self.cmd = new_cmd
	}

	/// IBat in range/heater fault check
	pub fn watchdog(
		&mut self,
		millivolts: MilliVolt,
		milliamps: MilliAmp,
		allow_undercurrent: AllowUndercurrent,
	) -> Result<(), FaultKind> {
		const PWM_MS_PERIOD: u8 = 20;
		//TODO: test this with oscilloscope
		/// ms it takes the hardware to reflect a change in pulse width
		const HW_REACTION_MS: u8 = 5;
		const WAIT_MS: u64 = (PWM_MS_PERIOD + HW_REACTION_MS) as u64;

		let dt = Instant::now() - self.change_time;
		if dt.as_millis() > WAIT_MS {
			match self.cmd {
				HeaterCmd::Off => {
					if milliamps > MilliAmp::new(100) {
						error!("Current above expected");
						Err(FaultKind::Overcurrent)
					} else {
						Ok(())
					}
				}
				HeaterCmd::On => match current_in_range(millivolts, milliamps) {
					Range::Hi => {
						error!("Current above expected");
						Err(FaultKind::Overcurrent)
					}
					Range::Lo => match allow_undercurrent {
						AllowUndercurrent::No => {
							error!("Current below expected");
							Err(FaultKind::Undercurrent)
						}
						AllowUndercurrent::Yes => Ok(()),
					},
					Range::Ok => Ok(()),
				},
			}
		} else {
			Ok(())
		}
	}
}

#[derive(defmt::Format, Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum HeaterCmd {
	#[default]
	Off,
	On,
}

impl Ord for HeaterCmd {
	fn cmp(&self, other: &Self) -> core::cmp::Ordering {
		match (self, other) {
			// self < other
			(HeaterCmd::Off, HeaterCmd::On) => core::cmp::Ordering::Less,
			// self > other
			(HeaterCmd::On, HeaterCmd::Off) => core::cmp::Ordering::Greater,
			_ => core::cmp::Ordering::Equal,
		}
	}
}

impl PartialOrd for HeaterCmd {
	fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

#[derive(defmt::Format, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
	Hi,
	Lo,
	Ok,
}

pub fn in_range_inclusive<V>(max: V, min: V, x: V) -> Range
where
	V: Copy + Ord,
{
	if x > max {
		Range::Hi
	} else if x < min {
		Range::Lo
	} else {
		Range::Ok
	}
}

pub fn expected_current(vbat: MilliVolt) -> MilliAmp {
	const TEST_MILLIVOLTS: u16 = 12_000;
	// TODO: test this
	const IMPERICAL_MILLIAMPS: u16 = 8_400;
	/// calculate system resistance (R = V / I)
	const R: u16 = TEST_MILLIVOLTS / IMPERICAL_MILLIAMPS;
	// I = V / R
	MilliAmp::new(Into::<u16>::into(vbat) / R)
}

pub fn current_in_range(vbat: MilliVolt, ibat: MilliAmp) -> Range {
	const MAX_DEVIATION: u16 = 200;
	let nom = expected_current(vbat);
	let max = MilliAmp::new(Into::<u16>::into(nom) + MAX_DEVIATION);
	let min = MilliAmp::new(Into::<u16>::into(nom) - MAX_DEVIATION);
	in_range_inclusive(max, min, ibat)
}

const PWM_CLOCK_HZ: f64 = 1_000_000.0;
const PWM_CLOCK_PERIOD: f64 = 1.0 / PWM_CLOCK_HZ;
const SERVO_HZ: f64 = 50.0;
const SERVO_PERIOD: f64 = 1.0 / SERVO_HZ;
const PWM_MAX_DUTY: u16 = (SERVO_PERIOD / PWM_CLOCK_PERIOD) as u16; // 20,000 = 20 ms
/// this is 1 / (13 + 1/3) of 20 milliseconds (1.5 millis aka 1500 micros)
pub const PWM_ZERO_OUTPUT: u16 = (PWM_MAX_DUTY as f64 / (13.0 + (1.0 / 3.0))) as u16;
pub const PWM_MAX_OUTPUT: u16 = PWM_MAX_DUTY / 10;

pub fn init_pwm_out(pwm: &mut SimplePwm<'static>) {
	pwm.disable();
	pwm.set_prescaler(Prescaler::Div16); // 1Mhz clock
	pwm.set_max_duty(PWM_MAX_DUTY);
	set_pwm(pwm, HeaterCmd::Off);
	pwm.enable();
	info!("init pwm");
}

pub fn percent_to_micros(pwm_on_percent: u8) -> u16 {
	let pwm_on_percent = pwm_on_percent as u16;

	const MULTIPLYER: u16 = (PWM_MAX_OUTPUT - PWM_ZERO_OUTPUT) / 100;

	let pwm_on_micros = pwm_on_percent * MULTIPLYER;

	pwm_on_micros.clamp(PWM_ZERO_OUTPUT, PWM_MAX_OUTPUT)
}

pub fn pwm_output_trim(setpoint: u16) -> u16 {
	// calibrate with o-scope
	const PLUS_TRIM: u16 = 16;
	PWM_MAX_DUTY - (setpoint + PLUS_TRIM)
}

pub fn set_pwm(pwm: &mut SimplePwm<'static>, cmd: HeaterCmd) {
	let duty = match cmd {
		HeaterCmd::Off => PWM_ZERO_OUTPUT,
		HeaterCmd::On => PWM_MAX_OUTPUT,
	};
	pwm.set_duty(0, pwm_output_trim(duty));
}
