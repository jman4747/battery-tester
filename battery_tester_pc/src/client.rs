use argh::FromArgs;
use bytes::BytesMut;
use pc_common::{SERVER_NAME, ServerCmd, write_ipc};
use thiserror::Error;
use tipsy::{Endpoint, ServerId};

#[tokio::main]
pub async fn main() -> Result<(), Error> {
	let cli: Cli = argh::from_env();
	let server_cmd: ServerCmd = cli.cmd.into();
	let mut client = Endpoint::connect(ServerId::new(SERVER_NAME))
		.await
		.map_err(|ioe| Error::Connect(ioe))?;
	let buf = BytesMut::with_capacity(512);
	let _buf = write_ipc(buf, &mut client, &server_cmd)
		.await
		.map_err(|ipce| Error::IPCWrite(ipce))?;
	Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
	#[error("can't connect to battery tester server")]
	Connect(#[source] std::io::Error),
	#[error("can't send message to server:\n{0:?}")]
	IPCWrite(#[source] tokio::io::Error),
}

#[derive(FromArgs, PartialEq, Eq, Clone)]
/// Battery tester client
pub struct Cli {
	#[argh(subcommand)]
	cmd: Subcommands,
}

#[derive(Debug, PartialEq, FromArgs, Eq, Clone)]
#[argh(subcommand)]
enum Subcommands {
	BatteryID(BatteryIdCmd),
	SerialDev(SerialDevCmd),
	SetCutoff(CutoffCmd),
	Start(StartCmd),
	/// cancel the test
	Cancel(CancelCmd),
	/// shutdown the server
	Shutdown(ShutdownCmd),
	ClearFault(ClearFaultCmd),
	AllowUndercurrent(UndercurrentResponse),
}

/// Undercurrent fault behavior
#[derive(Debug, PartialEq, FromArgs, Eq, Clone, Copy)]
#[argh(subcommand, name = "undercurrent")]
struct UndercurrentResponse {
	/// allow undercurrent
	#[argh(switch, short = 'a')]
	allow: bool,
}

/// Clear any faults
#[derive(Debug, PartialEq, FromArgs, Eq, Clone, Copy)]
#[argh(subcommand, name = "clear")]
struct ClearFaultCmd {}

/// start the test
#[derive(Debug, PartialEq, FromArgs, Eq, Clone, Copy)]
#[argh(subcommand, name = "start")]
struct StartCmd {}

/// cancel the test
#[derive(Debug, PartialEq, FromArgs, Eq, Clone, Copy)]
#[argh(subcommand, name = "cancel")]
struct CancelCmd {}

/// cancel the test and shutdown the server
#[derive(Debug, PartialEq, FromArgs, Eq, Clone, Copy)]
#[argh(subcommand, name = "shutdown")]
struct ShutdownCmd {}

/// set the voltage cutoff
#[derive(Debug, PartialEq, FromArgs, Eq, Clone, Copy)]
#[argh(subcommand, name = "cutoff")]
struct CutoffCmd {
	/// test cutoff voltage in millivolts
	#[argh(positional)]
	millivolts: u16,
}

/// set the battery ID
#[derive(Debug, PartialEq, FromArgs, Eq, Clone, Copy)]
#[argh(subcommand, name = "id")]
struct BatteryIdCmd {
	/// battery year
	#[argh(option, short = 'y')]
	year: u16,
	/// battery index
	#[argh(option, short = 'i')]
	index: u8,
}

/// set the name of the serial device.
#[derive(Debug, PartialEq, FromArgs, Eq, Clone)]
#[argh(subcommand, name = "device")]
struct SerialDevCmd {
	/// the name of the serical device /dev/tty-something or COM-something.
	#[argh(positional)]
	device_name: String,
}

impl From<Subcommands> for ServerCmd {
	fn from(value: Subcommands) -> Self {
		match value {
			Subcommands::BatteryID(battery_id_cmd) => Self::SetBatteryId(pc_common::BatteryID {
				year: battery_id_cmd.year,
				index: battery_id_cmd.index,
			}),
			Subcommands::SerialDev(serial_dev_cmd) => {
				Self::SetSerialDev(serial_dev_cmd.device_name.into_boxed_str())
			}
			Subcommands::SetCutoff(cutoff_cmd) => {
				Self::SetCutoffMillis(cutoff_cmd.millivolts.into())
			}
			Subcommands::Start(_start_cmd) => Self::StartTest,
			Subcommands::Cancel(_cancel_cmd) => Self::CancelTest,
			Subcommands::Shutdown(_shutdown_cmd) => Self::ShutDown,
			Subcommands::ClearFault(_clear_fault_cmd) => Self::ClearFault,
			Subcommands::AllowUndercurrent(resp) if resp.allow => Self::AllowUndercurrent,
			Subcommands::AllowUndercurrent(_resp) => Self::DisallowUndercurrent,
		}
	}
}
