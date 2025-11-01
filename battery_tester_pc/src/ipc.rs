use battery_tester_common::AllowUndercurrent;
use std::io::Write;
use tipsy::{Connection, Endpoint, ServerId};
use tokio::{
	io::AsyncReadExt,
	select,
	sync::{mpsc::Sender, oneshot::Receiver},
};

use futures::{pin_mut, stream::StreamExt};

use crate::{Event, Printer, SERVER_NAME, ServerCmd};

async fn for_each_conn(
	conn_res: Result<Connection, std::io::Error>,
	event_tx: &Sender<Event>,
	mut printer: Printer,
) {
	const STATIC_BUF_SIZE: usize = 512;
	match conn_res {
		Ok(mut stream) => {
			let cmd: ServerCmd = {
				let to_read = match stream.read_u32().await.map(|n| n as usize) {
					Ok(r) => r,
					Err(e) => {
						printer.buf(|tv| write!(tv, "bad command: {e:?}")).await;
						return;
					}
				};
				if to_read > STATIC_BUF_SIZE {
					let mut buf = Vec::with_capacity(to_read);
					let _ = stream.read_to_end(&mut buf).await.unwrap();
					postcard::from_bytes(&buf[..to_read]).unwrap()
				} else {
					let mut stat_buf = [0u8; STATIC_BUF_SIZE];
					let mut buf = &mut stat_buf[..to_read];
					let _ = stream.read_exact(&mut buf).await.unwrap();
					postcard::from_bytes(&buf).unwrap()
				}
			};
			match cmd {
				ServerCmd::SetBatteryId(battery_id) => event_tx.send(Event::BattID(battery_id)),
				ServerCmd::SetSerialDev(dev) => event_tx.send(Event::SetSerialDevice(dev)),
				ServerCmd::SetCutoffMillis(millivolts) => {
					event_tx.send(Event::SetCutoff(millivolts))
				}
				ServerCmd::StartTest => event_tx.send(Event::StartTest),
				ServerCmd::CancelTest => event_tx.send(Event::CancelTest),
				ServerCmd::ShutDown => event_tx.send(Event::Shutdown),
				ServerCmd::ClearFault => event_tx.send(Event::ClearFault),
				ServerCmd::AllowUndercurrent => {
					event_tx.send(Event::UnderCurrentResponse(AllowUndercurrent::Yes))
				}
				ServerCmd::DisallowUndercurrent => {
					event_tx.send(Event::UnderCurrentResponse(AllowUndercurrent::No))
				}
			}
			.await
			.unwrap();
		}
		Err(e) => {
			printer
				.buf(|tv| write!(tv, "Error receiving connection: {:?}", e))
				.await
		}
	}
}

pub async fn ipc_task(
	event_tx: Sender<Event>,
	printer: Printer,
	mut ipc_shutdown_rx: Receiver<()>,
) -> Result<(), std::io::Error> {
	let id = ServerId::new(SERVER_NAME);
	let incoming_stream = Endpoint::new(id, tipsy::OnConflict::Overwrite)?.incoming()?;
	// .for_each(|conn_res| for_each_conn(conn_res, &event_tx, &print_tx));
	pin_mut!(incoming_stream);
	loop {
		select! {
			conn_op = incoming_stream.next() => {
				match conn_op {
					Some(conn_res) => {
						for_each_conn(conn_res, &event_tx, printer.clone()).await
					}
					None => break,
				}
			}
			_ = &mut ipc_shutdown_rx => {
				break;
			}
		}
	}
	println!("exiting ipc_task");
	Ok(())
}
