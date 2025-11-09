use std::io::Write;
use std::path::PathBuf;

use battery_tester_common::{FaultKind, MilliVolt};
use pc_common::{
	BatteryID, Cli, ComCmd, Error, Event, FileCmd, Mode, Print, Printer, SaveData, TestState,
	end_test_command, files::file_task, idle_command, ipc::ipc_task, print_task,
	serial::serial_com_task, testing_command, volts_command,
};
use tokio::{
	fs::{File, OpenOptions},
	sync::{
		mpsc::{self, Receiver, Sender},
		oneshot,
	},
};

#[tokio::main]
async fn main() -> Result<(), Error> {
	let cli: Cli = argh::from_env();
	let output_dir = if cli.output_directory.is_dir() {
		cli.output_directory
	} else {
		return Err(Error::OutputPathIsDir(
			cli.output_directory.into_boxed_path(),
		));
	};

	// cross task comms
	let (print_tx, print_rx) = mpsc::channel::<Print>(16);
	let (program_event_tx, program_event_rx) = mpsc::channel::<Event>(8);
	let (file_cmd_tx, file_cmd_rx) = mpsc::channel::<FileCmd>(8);
	let (com_cmd_tx, com_cmd_rx) = mpsc::channel::<ComCmd>(8);
	let (ipc_shutdown_tx, ipc_shutdown_rx) = oneshot::channel();

	// println!() replacement
	let print_task_hanle = tokio::spawn(print_task(print_rx));
	let printer = Printer::new(print_tx);

	// main control loop
	let program_task_handle = tokio::spawn(program_event_task(
		program_event_rx,
		file_cmd_tx.clone(),
		com_cmd_tx.clone(),
		output_dir,
		printer.clone(),
		ipc_shutdown_tx,
	));
	let com_task_handle = tokio::spawn(serial_com_task(
		program_event_tx.clone(),
		com_cmd_rx,
		printer.clone(),
	));
	let file_task_handle = tokio::spawn(file_task(program_event_tx.clone(), file_cmd_rx));
	let ipc_task_handle = tokio::spawn(ipc_task(
		program_event_tx.clone(),
		printer.clone(),
		ipc_shutdown_rx,
	));
	// TODO; handle JoinErr?
	let (_prog_res, _com_res, _file_res, _print_res, _ipc_res) = tokio::join!(
		program_task_handle,
		com_task_handle,
		file_task_handle,
		print_task_hanle,
		ipc_task_handle
	);
	print!("exiting...");
	Ok(())
}

async fn program_event_task(
	mut rx: Receiver<Event>,
	file_cmd_tx: Sender<FileCmd>,
	com_cmd_tx: Sender<ComCmd>,
	mut output_dir: PathBuf,
	mut printer: Printer,
	ipc_shutdown_tx: oneshot::Sender<()>,
) {
	printer.stat("program started...").await;
	let mut state = TestState::default();
	let mut mode = Mode::default();
	loop {
		mode = match mode {
			Mode::Setup => {
				setup(
					&mut state,
					&mut rx,
					&com_cmd_tx,
					&file_cmd_tx,
					&mut output_dir,
					&mut printer,
				)
				.await
			}
			Mode::WaitForBattery => {
				wait_for_battery(
					&mut state,
					&mut rx,
					&com_cmd_tx,
					&file_cmd_tx,
					&mut output_dir,
					&mut printer,
				)
				.await
			}
			Mode::WaitForUsrStart => {
				wait_for_usr_start(
					&mut state,
					&mut rx,
					&file_cmd_tx,
					&mut output_dir,
					&mut printer,
				)
				.await
			}
			Mode::Testing => {
				testing(&mut state, &mut rx, &com_cmd_tx, &file_cmd_tx, &mut printer).await
			}
			Mode::EndTest => end_test(&mut state, &com_cmd_tx, &file_cmd_tx, &mut printer).await,
			Mode::Paused => todo!(),
			Mode::Shutdown => {
				shutdown(com_cmd_tx, file_cmd_tx, printer, ipc_shutdown_tx).await;
				break;
			}
			Mode::CommDC => comm_dc(&mut state, &file_cmd_tx, &mut printer).await,
			Mode::Fault => {
				fault(
					&mut state,
					&mut rx,
					&com_cmd_tx,
					&file_cmd_tx,
					&mut output_dir,
					&mut printer,
				)
				.await
			}
		};
	}
}

async fn shutdown(
	com_cmd_tx: Sender<ComCmd>,
	file_cmd_tx: Sender<FileCmd>,
	printer: Printer,
	ipc_shutdown_tx: oneshot::Sender<()>,
) {
	com_cmd_tx
		.send(ComCmd::BICommand(idle_command()))
		.await
		.unwrap();
	file_cmd_tx.send(FileCmd::CloseFile).await.unwrap();
	file_cmd_tx.send(FileCmd::Shutdown).await.unwrap();
	com_cmd_tx.send(ComCmd::Shutdown).await.unwrap();
	ipc_shutdown_tx.send(()).unwrap();
	printer.shutdown().await;
}

async fn comm_dc(
	state: &mut TestState,
	file_cmd_tx: &Sender<FileCmd>,
	printer: &mut Printer,
) -> Mode {
	printer.stat("serial comms disconnected").await;
	file_cmd_tx.send(FileCmd::CloseFile).await.unwrap();
	state.end_test();
	Mode::Setup
}

async fn end_test(
	state: &mut TestState,
	com_cmd_tx: &Sender<ComCmd>,
	file_cmd_tx: &Sender<FileCmd>,
	printer: &mut Printer,
) -> Mode {
	com_cmd_tx
		.send(ComCmd::BICommand(end_test_command()))
		.await
		.unwrap();
	file_cmd_tx.send(FileCmd::CloseFile).await.unwrap();
	printer.stat("ending test...").await;
	state.end_test();
	Mode::Setup
}

async fn testing(
	state: &mut TestState,
	event_rx: &mut Receiver<Event>,
	com_cmd_tx: &Sender<ComCmd>,
	file_cmd_tx: &Sender<FileCmd>,
	printer: &mut Printer,
) -> Mode {
	printer.stat("starting test...").await;
	com_cmd_tx
		.send(ComCmd::BICommand(testing_command(
			state.get_allow_undercurrent(),
		)))
		.await
		.unwrap();
	loop {
		let event = match event_rx.recv().await {
			Some(e) => e,
			None => return Mode::Shutdown,
		};
		match event {
			Event::SetCutoff(millivolts) => new_cutoff(state, millivolts, printer).await,
			Event::ComReply(reply) => match reply.fault {
				Err(f) => {
					match f.kind {
						FaultKind::I2C(i2ce) => {
							printer.buf(|b| write!(b, "I2C Fault:\n{i2ce:?}")).await;
						}
						FaultKind::Undercurrent => {
							printer.stat("Heater undercurret/not present!").await;
						}
						FaultKind::NoBattery => {
							printer.stat("Battery Disconnected!").await;
						}
						FaultKind::Overcurrent => {
							printer.stat("Heater overcurrent!").await;
						}
					}
					break Mode::Fault;
				}
				Ok(()) => match reply.measurement {
					Some(m) if m.vbat > state.cutoff() => {
						// keep testing
						file_cmd_tx
							.send(FileCmd::Push(SaveData {
								millivolts: m.vbat,
								milliamps: m.ibat,
								t_start: m.t_start,
								duration: m.duration,
							}))
							.await
							.unwrap();
					}
					Some(_m) => break Mode::EndTest, // at cutoff, stop testing
					None => {
						// no new data this time, keep testing
					}
				},
			},
			Event::CommDc => break Mode::CommDC,
			Event::StartTest => {
				printer.stat("already testing").await;
			}
			Event::CancelTest => break Mode::EndTest,
			Event::Shutdown => break Mode::Shutdown,
			Event::SetSerialDevice(_dev_id) => {
				printer
					.stat("can't change serial device while testing")
					.await;
			}
			Event::BattID(_battery_id) => {
				printer.stat("can't change battery ID while testing").await;
			}
			Event::FileError => break Mode::EndTest,
			Event::ClearFault => {
				printer.stat("no fault to clear").await;
			}
			Event::UnderCurrentResponse(allow_undercurrent) => {
				state.set_allow_undercurrent(allow_undercurrent)
			}
		}
	}
}

async fn wait_for_usr_start(
	state: &mut TestState,
	event_rx: &mut Receiver<Event>,
	file_cmd_tx: &Sender<FileCmd>,
	output_dir: &mut PathBuf,
	printer: &mut Printer,
) -> Mode {
	printer.stat("waiting for user to start test...").await;
	loop {
		let event = match event_rx.recv().await {
			Some(e) => e,
			None => return Mode::Shutdown,
		};
		match event {
			Event::BattID(battery_id) => match new_file(battery_id, output_dir, printer).await {
				Ok(file) => {
					file_cmd_tx.send(FileCmd::NewFile(file)).await.unwrap();
					state.new_batt_id(battery_id)
				}
				Err(e) => {
					printer
						.buf(|tv| write!(tv, "can't create new output file:\n{e}"))
						.await;
					break Mode::EndTest;
				}
			},
			Event::StartTest => break Mode::Testing,
			Event::ComReply(reply) => match reply.fault {
				Ok(()) => {
					if let Some(m) = reply.measurement {
						// double check that the battery is over cutoff
						if !(m.vbat > state.cutoff()) {
							break Mode::WaitForBattery;
						}
					}
				}
				Err(f) => {
					printer.buf(|tv| write!(tv, "fault:\n{f:?}")).await;
					break Mode::Fault;
				}
			},
			Event::SetCutoff(millivolts) => new_cutoff(state, millivolts, printer).await,
			Event::CommDc => break Mode::CommDC,
			Event::CancelTest => break Mode::EndTest,
			Event::SetSerialDevice(_) => {
				// TODO: warn user
			}
			Event::Shutdown => break Mode::Shutdown,
			Event::FileError => break Mode::EndTest,
			Event::ClearFault => {
				printer.stat("no fault to clear").await;
			}
			Event::UnderCurrentResponse(allow_undercurrent) => {
				state.set_allow_undercurrent(allow_undercurrent)
			}
		}
	}
}

async fn wait_for_battery(
	state: &mut TestState,
	event_rx: &mut Receiver<Event>,
	com_cmd_tx: &Sender<ComCmd>,
	file_cmd_tx: &Sender<FileCmd>,
	output_dir: &mut PathBuf,
	printer: &mut Printer,
) -> Mode {
	printer.stat("waiting for battery connection...").await;
	com_cmd_tx
		.send(ComCmd::BICommand(volts_command()))
		.await
		.unwrap();
	loop {
		let event = match event_rx.recv().await {
			Some(e) => e,
			None => return Mode::Shutdown,
		};
		match event {
			Event::BattID(battery_id) => match new_file(battery_id, output_dir, printer).await {
				Ok(file) => {
					file_cmd_tx.send(FileCmd::NewFile(file)).await.unwrap();
					state.new_batt_id(battery_id)
				}
				Err(e) => {
					printer
						.buf(|tv| write!(tv, "can't create new output file:\n{e}"))
						.await;
					break Mode::EndTest;
				}
			},
			Event::SetCutoff(millivolts) => new_cutoff(state, millivolts, printer).await,
			Event::StartTest => {
				printer
					.stat("can't start test while waiting for battery")
					.await;
			}
			Event::CommDc => {
				break Mode::CommDC;
			}
			Event::ComReply(reply) => match reply.fault {
				Ok(()) => {
					if let Some(m) = reply.measurement {
						if m.vbat > state.cutoff() {
							// battery connected, wait for user to start
							break Mode::WaitForUsrStart;
						} else {
							// battery not connected yet
						}
					}
				}
				Err(f) => {
					printer.buf(|tv| write!(tv, "fault:\n{f:?}")).await;
					break Mode::Fault;
				}
			},
			Event::CancelTest => break Mode::EndTest,
			Event::SetSerialDevice(_) => {
				printer
					.stat("can't change serial device while waiting for battery")
					.await;
			}
			Event::Shutdown => break Mode::Shutdown,
			Event::FileError => break Mode::EndTest,
			Event::ClearFault => {
				printer.stat("no fault to clear").await;
			}
			Event::UnderCurrentResponse(allow_undercurrent) => {
				state.set_allow_undercurrent(allow_undercurrent)
			}
		}
	}
}

async fn fault(
	state: &mut TestState,
	event_rx: &mut Receiver<Event>,
	com_cmd_tx: &Sender<ComCmd>,
	file_cmd_tx: &Sender<FileCmd>,
	output_dir: &mut PathBuf,
	printer: &mut Printer,
) -> Mode {
	com_cmd_tx
		.send(ComCmd::BICommand(idle_command()))
		.await
		.unwrap();
	printer.stat("ending test, clear fault to continue").await;
	file_cmd_tx.send(FileCmd::CloseFile).await.unwrap();
	state.end_test();
	loop {
		let event = match event_rx.recv().await {
			Some(e) => e,
			None => return Mode::Shutdown,
		};
		match event {
			Event::BattID(battery_id) => match new_file(battery_id, output_dir, printer).await {
				Ok(file) => {
					file_cmd_tx.send(FileCmd::NewFile(file)).await.unwrap();
					state.new_batt_id(battery_id);
				}
				Err(e) => {
					printer
						.buf(|tv| write!(tv, "can't create new output file:\n{e}"))
						.await;
					state.end_test();
				}
			},
			Event::SetSerialDevice(dev_id) => {
				printer
					.buf(|tv| write!(tv, "setting device name to: {}", &dev_id))
					.await;
				com_cmd_tx
					.send(ComCmd::NewDeviceName(dev_id))
					.await
					.unwrap();
			}
			Event::SetCutoff(millivolts) => new_cutoff(state, millivolts, printer).await,
			Event::ComReply(reply) => match reply.fault {
				Ok(()) => {
					printer.stat("fault cleared").await;
					break;
				}
				Err(_f) => {
					// still getting a fault
				}
			},
			Event::Shutdown => return Mode::Shutdown,
			Event::CommDc => {
				printer
					.stat("lost serial comms with battery interface")
					.await;
				return Mode::Setup;
			}
			Event::StartTest => {
				printer
					.stat("cant't start test until fault is cleared")
					.await;
			}
			Event::CancelTest => {
				// TODO: warn user
			}
			Event::FileError => {}
			Event::ClearFault => {
				com_cmd_tx.send(ComCmd::ClearFault).await.unwrap();
				// dont break or return because we want an OK(()) reply from BI
			}
			Event::UnderCurrentResponse(allow_undercurrent) => {
				state.set_allow_undercurrent(allow_undercurrent)
			}
		}
	}
	Mode::Setup
}
async fn setup(
	state: &mut TestState,
	event_rx: &mut Receiver<Event>,
	com_cmd_tx: &Sender<ComCmd>,
	file_cmd_tx: &Sender<FileCmd>,
	output_dir: &mut PathBuf,
	printer: &mut Printer,
) -> Mode {
	printer
		.stat("setup: please set battery ID and tester serial port device name")
		.await;
	com_cmd_tx
		.send(ComCmd::BICommand(idle_command()))
		.await
		.unwrap();
	printer.buf(|tv| write!(tv, "{:?}", &state)).await;
	loop {
		let event = match event_rx.recv().await {
			Some(e) => e,
			None => return Mode::Shutdown,
		};
		match event {
			Event::BattID(battery_id) => match new_file(battery_id, output_dir, printer).await {
				Ok(file) => {
					file_cmd_tx.send(FileCmd::NewFile(file)).await.unwrap();
					state.new_batt_id(battery_id);
					if state.ready_for_battery() {
						break Mode::WaitForBattery;
					} else {
						printer.buf(|tv| write!(tv, "{:?}", &state)).await;
					}
				}
				Err(e) => {
					printer
						.buf(|tv| write!(tv, "can't create new output file:\n{e}"))
						.await;
					state.end_test();
				}
			},
			Event::SetSerialDevice(dev_id) => {
				printer
					.buf(|tv| write!(tv, "setting device name to: {}", &dev_id))
					.await;
				com_cmd_tx
					.send(ComCmd::NewDeviceName(dev_id.clone()))
					.await
					.unwrap();
				state.new_device_name(dev_id);
				printer.buf(|tv| write!(tv, "{:?}", &state)).await;
			}
			Event::SetCutoff(millivolts) => new_cutoff(state, millivolts, printer).await,
			Event::ComReply(reply) => match reply.fault {
				Ok(()) => {
					if !state.got_first_reply() {
						state.set_first_reply();
						printer.buf(|tv| write!(tv, "{:?}", &state)).await;
					}
					if state.ready_for_battery() {
						break Mode::WaitForBattery;
					}
				}
				Err(f) => {
					// got_ok_reply = false;
					match f.kind {
						FaultKind::I2C(i2ce) => {
							printer.buf(|b| write!(b, "I2C Fault:\n{i2ce:?}")).await;
						}
						FaultKind::Undercurrent => {
							printer.stat("Heater undercurret/not present!").await;
						}
						FaultKind::NoBattery => {
							printer.stat("Battery Disconnected!").await;
						}
						FaultKind::Overcurrent => {
							printer.stat("Heater overcurrent!").await;
						}
					}
					break Mode::Fault;
				}
			},
			Event::Shutdown => break Mode::Shutdown,
			Event::CommDc => state.unset_first_reply(),
			Event::StartTest => {
				printer.stat("cant't start test during setup").await;
			}
			Event::CancelTest => {}
			Event::FileError => state.end_test(),
			Event::ClearFault => {
				printer.stat("no fault to clear").await;
			}
			Event::UnderCurrentResponse(allow_undercurrent) => {
				state.set_allow_undercurrent(allow_undercurrent)
			}
		}
	}
}

async fn new_cutoff(state: &mut TestState, millivolts: MilliVolt, printer: &mut Printer) {
	state.new_cutoff(millivolts);
	printer
		.buf(|tv| write!(tv, "new cutoff voltage (millivolts): {millivolts}"))
		.await;
}

async fn new_file(
	battery_id: BatteryID,
	output_dir: &mut PathBuf,
	printer: &mut Printer,
) -> tokio::io::Result<File> {
	let now = chrono::Local::now().format("%Y%m%d_%TUTC%Z");
	let battery_year = battery_id.year;
	let battery_idx = battery_id.index;
	let file_name = format!("{battery_year}-{battery_idx}-{now}.tsv");
	output_dir.push(file_name);
	let res = OpenOptions::new()
		.write(true)
		.read(true)
		.append(true)
		.create_new(true)
		.open(&output_dir)
		.await;
	if res.is_ok() {
		printer
			.buf(|tv| write!(tv, "created new file at: {:?}", &output_dir))
			.await;
	}
	output_dir.pop();
	res
}
