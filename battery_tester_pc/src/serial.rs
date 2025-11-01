use battery_tester_common::{BIReply, BiCommand};
use bytes::BytesMut;
use tokio::{
	io::AsyncReadExt,
	select,
	sync::mpsc::{Receiver, Sender},
	time::MissedTickBehavior,
};
use tokio_serial::{SerialPort, SerialPortBuilderExt, SerialStream};

use crate::{
	ComCmd, DEFALT_BAUD, Event, INCOMING_MAX_SIZE, OUTGOING_MAX_SIZE, Printer, clear_fault_command,
	idle_command,
};

pub async fn serial_com_task(
	event_tx: Sender<Event>,
	mut com_cmd_rx: Receiver<ComCmd>,
	mut printer: Printer,
) {
	use std::io::Write;
	let mut daq_serial = loop {
		match com_cmd_rx.recv().await {
			Some(ComCmd::NewDeviceName(dev_name)) => match connect(dev_name.as_ref()).await {
				Ok(ds) => break ds,
				Err(e) => {
					printer
						.buf(|tv| {
							write!(
								tv,
								"can't make initial connection to: {dev_name} due to:\n{e}"
							)
						})
						.await
				}
			},
			Some(ComCmd::Shutdown) => {
				println!("exiting serial_com_task");
				return;
			}
			None => return,
			_ => {}
		}
	};
	use tokio::time::{self, Duration};
	// we send at 2Hz
	let mut tx_interval = time::interval(Duration::from_millis(500));
	tx_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
	let mut incoming_buf: BytesMut = BytesMut::with_capacity(INCOMING_MAX_SIZE);
	let mut bi_command = BiCommand::default();
	loop {
		let new_cmd: Option<ComCmd> = select! {
			cmd = com_cmd_rx.recv() => {
				printer.buf(|tv| write!(tv, "command: {:?}", &cmd)).await;
				cmd
			}
			serial_resp = serial_read_response(&mut daq_serial, &mut incoming_buf) => {
				match serial_resp {
					Ok(reply) => {
						event_tx.send(Event::ComReply(reply)).await.unwrap();
						None
					}
					Err(e) => {
						printer.buf(|tv| write!(tv, "serial comm error when reading BI response:\n{e}")).await;
						event_tx.send(Event::CommDc).await.unwrap();
						None
					}
				}
			}
			_ = tx_interval.tick() => {
				match serial_write_command(&mut daq_serial, &bi_command).await {
					Ok(_) => None,
					Err(e) => {
						printer.buf(|tv| write!(tv, "serial comm error when writing BI command on regular interval:\n{e}")).await;
						event_tx.send(Event::CommDc).await.unwrap();
						None
					}
				}
			}
		};

		match new_cmd {
			Some(ComCmd::BICommand(new_bi_command)) => {
				bi_command = new_bi_command;
				if let Err(serial_err) = serial_write_command(&mut daq_serial, &bi_command).await {
					printer
						.buf(|tv| {
							write!(
								tv,
								"serial comm error when writing BI command:\n{serial_err}"
							)
						})
						.await;
					event_tx.send(Event::CommDc).await.unwrap();
				}
			}
			Some(ComCmd::NewDeviceName(dev_name)) => {
				daq_serial = match connect(dev_name.as_ref()).await {
					Ok(ds) => ds,
					Err(tse) => {
						printer
							.buf(|tv| {
								write!(
									tv,
									"can't connect to device: {} serical comm error: {tse}",
									&dev_name
								)
							})
							.await;
						event_tx.send(Event::CommDc).await.unwrap();
						continue;
					}
				};
			}
			Some(ComCmd::Shutdown) => {
				let command = idle_command();
				let _ = serial_write_command(&mut daq_serial, &command).await;
				break;
			}
			Some(ComCmd::ClearFault) => {
				let command = clear_fault_command();
				if let Err(serial_err) = serial_write_command(&mut daq_serial, &command).await {
					printer
						.buf(|tv| {
							write!(tv, "serial comm error when clearing fault:\n{serial_err}")
						})
						.await;
					event_tx.send(Event::CommDc).await.unwrap();
				}
			}
			None => {}
		}
	}
	println!("exiting serial_com_task");
}

async fn connect(dev_name: &str) -> Result<SerialStream, tokio_serial::Error> {
	let mut daq_serial = tokio_serial::new(dev_name, DEFALT_BAUD)
		.data_bits(tokio_serial::DataBits::Eight)
		.stop_bits(tokio_serial::StopBits::One)
		.open_native_async()?;

	daq_serial.set_exclusive(false)?;
	daq_serial.clear(tokio_serial::ClearBuffer::All)?;
	Ok(daq_serial)
}

async fn serial_write_command(
	serial_write: &mut SerialStream,
	ctrl_word: &BiCommand,
) -> Result<(), tokio_serial::Error> {
	debug_assert!(OUTGOING_MAX_SIZE < u8::MAX as usize);
	let mut outgoing_buf: [u8; OUTGOING_MAX_SIZE] = [0u8; OUTGOING_MAX_SIZE];
	let outgoing = postcard::to_slice(ctrl_word, &mut outgoing_buf[..]).unwrap();
	serial_write_general(&outgoing, serial_write).await
}

async fn serial_write_general(
	outgoing: &[u8],
	serial_write: &mut SerialStream,
) -> Result<(), tokio_serial::Error> {
	use tokio::io::AsyncWriteExt;
	let total = outgoing.len() as u8;
	serial_write.write_u8(total).await?;
	let total = total as usize;
	let mut remaining = total;
	while remaining > 0 {
		remaining -= serial_write
			.write(&outgoing[total - remaining..total])
			.await?;
	}
	Ok(())
}

async fn serial_read_response(
	serial_read: &mut SerialStream,
	incoming_buf: &mut BytesMut,
) -> Result<BIReply, tokio_serial::Error> {
	incoming_buf.clear();
	// first byte is message len
	assert!(INCOMING_MAX_SIZE < u8::MAX as usize);
	let message_total = serial_read.read_u8().await.map(|len| len as usize)?;

	let mut remaining = message_total;
	while remaining > 0 {
		remaining -= serial_read.read_buf(incoming_buf).await?;
	}
	let response: BIReply = postcard::from_bytes(&incoming_buf[..message_total]).unwrap();
	Ok(response)
}
