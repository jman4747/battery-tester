use argh::FromArgs;
use battery_tester_common::{
	AllowUndercurrent, BIReply, BiCommand, ClearFault, LoadState, MilliAmp, MilliVolt, Reset,
};
use bytes::BytesMut;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tinyvec::{ArrayVec, TinyVec, tiny_vec};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{Receiver, Sender};

pub mod files;
pub mod ipc;
pub mod serial;

pub const OUTGOING_MAX_SIZE: usize = BiCommand::POSTCARD_MAX_SIZE;
pub const INCOMING_MAX_SIZE: usize = BIReply::POSTCARD_MAX_SIZE;
pub const DEFALT_BAUD: u32 = 230400;
pub const DEFAULT_CUTOFF_MILLIV: u16 = 11_000;
pub const DEFAULT_DISCONNECT_MILLIV: u16 = 1_000;
pub const SERVER_NAME: &str = "battery-tester-server";

#[derive(Debug, Clone)]
pub struct Printer {
	sender: Sender<Print>,
}

impl Printer {
	pub fn new(sender: Sender<Print>) -> Self {
		Self { sender: sender }
	}

	pub async fn shutdown(self) {
		self.sender.send(Print::Shutdown).await.unwrap();
	}

	pub async fn stat(&self, msg: &'static str) {
		self.sender.send(Print::Static(msg)).await.unwrap()
	}

	pub async fn buf<F>(&mut self, mut f: F)
	where
		F: FnMut(&mut TinyVec<[u8; 128]>) -> Result<(), std::io::Error>,
	{
		let mut buf = tiny_vec!([u8; 128]);
		let _ = f(&mut buf);
		match buf {
			TinyVec::Inline(array_vec) => self.sender.send(Print::Dyn(array_vec)).await.unwrap(),
			TinyVec::Heap(items) => self
				.sender
				.send(Print::Aloc(items.into_boxed_slice()))
				.await
				.unwrap(),
		}
	}
}

/// Due to how postcard::to_extend works, we return the buffer after clearing it
/// instead of just takeing a mutable reference.
pub async fn write_ipc<T>(
	out_buf: BytesMut,
	stream: &mut tipsy::Connection,
	cmd: &T,
) -> Result<BytesMut, tokio::io::Error>
where
	T: serde::Serialize + ?Sized,
{
	// then we serialize the command (cmd) extending (appending to) the buffer
	let mut serialized = postcard::to_extend(cmd, out_buf).unwrap();

	// next we get the length of everything we just added after the u32 message length
	let out_len = serialized.len() as u32;

	// println!("outbuf len = {:x?}", &serialized[..4]);
	// println!("outbuf content: {:x?}", &serialized[4..]);

	stream.write_u32(out_len).await?;
	stream.write_all(&serialized).await?;
	stream.flush().await?;
	serialized.clear();
	Ok(serialized)
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Print {
	Static(&'static str),
	Dyn(ArrayVec<[u8; 128]>),
	Aloc(Box<[u8]>),
	Shutdown,
}

impl Print {
	pub fn as_bytes(&self) -> &[u8] {
		match self {
			Print::Static(sstr) => sstr.as_bytes(),
			Print::Aloc(bstr) => bstr.as_ref(),
			Print::Shutdown => b"\n",
			Print::Dyn(array_vec) => array_vec.as_ref(),
		}
	}
}

pub async fn print_task(mut print_rx: Receiver<Print>) {
	let mut stdout = tokio::io::stdout();
	while let Some(msg) = print_rx.recv().await {
		if let Print::Shutdown = msg {
			break;
		}
		stdout.write_all(msg.as_bytes()).await.unwrap();
		stdout.write_u8(b'\n').await.unwrap();
		stdout.flush().await.unwrap();
	}
	println!("exiting print_task");
}

#[derive(FromArgs, PartialEq, Eq, Clone)]
/// Battery tester server
pub struct Cli {
	#[argh(positional)]
	pub output_directory: std::path::PathBuf,
}

#[derive(Debug, Error)]
pub enum Error {
	#[error("given output directory: {0:?} isn't a directory (folder)")]
	OutputPathIsDir(Box<std::path::Path>),
}

#[derive(Debug, Default, PartialEq, Eq, Copy, Clone)]
pub enum Mode {
	#[default]
	/// Wait for device ID, batt ID, BI replies start
	Setup,
	/// Wait for voltage to show greater than cutoff
	WaitForBattery,
	/// Wait for user to send start command
	WaitForUsrStart,
	/// Testing, waiting for voltage <= cutoff
	Testing,
	/// User paused test
	Paused,
	/// User shutdown server
	Shutdown,
	/// Test ended
	EndTest,
	/// Serial comms not working
	CommDC,
	Fault,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TestState {
	cutoff: MilliVolt,
	battery_id: Option<BatteryID>,
	device_name: Option<Box<str>>,
	first_reply: bool,
	allow_undercurrent: AllowUndercurrent,
}

impl Default for TestState {
	fn default() -> Self {
		Self {
			cutoff: DEFAULT_CUTOFF_MILLIV.into(),
			battery_id: Default::default(),
			device_name: Default::default(),
			first_reply: false,
			allow_undercurrent: Default::default(),
		}
	}
}

impl TestState {
	pub fn new_cutoff(&mut self, millivolts: MilliVolt) {
		self.cutoff = millivolts;
	}

	pub fn new_batt_id(&mut self, battery_id: BatteryID) {
		self.battery_id = Some(battery_id)
	}

	pub fn new_device_name(&mut self, device_name: Box<str>) {
		self.device_name = Some(device_name)
	}

	pub fn set_first_reply(&mut self) {
		self.first_reply = true
	}

	pub fn unset_first_reply(&mut self) {
		self.first_reply = false
	}

	pub fn got_first_reply(&self) -> bool {
		self.first_reply
	}

	pub fn cutoff(&self) -> MilliVolt {
		self.cutoff
	}

	pub fn battery_id(&self) -> Option<BatteryID> {
		self.battery_id
	}

	pub fn end_test(&mut self) {
		self.battery_id = None;
		self.first_reply = false;
	}

	pub fn ready_for_battery(&self) -> bool {
		self.battery_id.is_some() && self.first_reply && self.device_name.is_some()
	}

	pub fn get_allow_undercurrent(&self) -> AllowUndercurrent {
		self.allow_undercurrent
	}
	pub fn set_allow_undercurrent(&mut self, allow_undercurrent: AllowUndercurrent) {
		self.allow_undercurrent = allow_undercurrent
	}
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum ServerCmd {
	SetBatteryId(BatteryID),
	SetSerialDev(Box<str>),
	SetCutoffMillis(MilliVolt),
	StartTest,
	//TODO: PauseTest,
	CancelTest,
	ShutDown,
	ClearFault,
	AllowUndercurrent,
	DisallowUndercurrent,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Event {
	/// User sent battery ID
	BattID(BatteryID),
	/// User set device name
	SetSerialDevice(Box<str>),
	/// User set cutoff voltage
	SetCutoff(MilliVolt),
	/// User wants to start test
	StartTest,
	/// Com not getting replies
	CommDc,
	/// Com reply
	ComReply(BIReply),
	/// User canceled battery ID
	CancelTest,
	/// User sent shutdown command
	Shutdown,
	/// Updates can't be written to the file  
	FileError,
	// /// IPC dissconnected
	// IpcError,
	/// Clear fault
	ClearFault,
	/// Allow current to be below expected or not
	UnderCurrentResponse(AllowUndercurrent),
}

#[derive(Debug)]
pub enum FileCmd {
	NewFile(tokio::fs::File),
	CloseFile,
	Shutdown,
	Push(SaveData),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct SaveData {
	pub millivolts: MilliVolt,
	pub milliamps: MilliAmp,
	pub dt: u64,
	pub duration: u64,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ComCmd {
	NewDeviceName(Box<str>),
	BICommand(BiCommand),
	Shutdown,
	ClearFault,
}

pub fn idle_command() -> BiCommand {
	BiCommand {
		load: LoadState::Off,
		clear_fault: ClearFault::No,
		reset: Reset::No,
		allow_undercurrent: AllowUndercurrent::No,
	}
}

pub fn end_test_command() -> BiCommand {
	BiCommand {
		load: LoadState::Off,
		clear_fault: ClearFault::No,
		reset: Reset::Yes,
		allow_undercurrent: AllowUndercurrent::No,
	}
}

pub fn volts_command() -> BiCommand {
	BiCommand {
		load: LoadState::Off,
		clear_fault: ClearFault::No,
		reset: Reset::No,
		allow_undercurrent: AllowUndercurrent::No,
	}
}

pub fn testing_command(allow_undercurrent: AllowUndercurrent) -> BiCommand {
	BiCommand {
		load: LoadState::On,
		clear_fault: ClearFault::No,
		reset: Reset::No,
		allow_undercurrent,
	}
}

fn clear_fault_command() -> BiCommand {
	BiCommand {
		load: LoadState::Off,
		clear_fault: ClearFault::Yes,
		reset: Reset::No,
		allow_undercurrent: AllowUndercurrent::No,
	}
}

#[derive(Debug, PartialEq, Eq, Copy, Clone, Serialize, Deserialize, MaxSize)]
pub struct BatteryID {
	pub year: u16,
	pub index: u8,
}
