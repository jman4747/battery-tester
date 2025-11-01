#![no_std]

use defmt::Format;
use nutype::nutype;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

pub const COMMAND_MAX_SIZE: usize = BiCommand::POSTCARD_MAX_SIZE;
pub const REPLY_MAX_SIZE: usize = BIReply::POSTCARD_MAX_SIZE;

#[nutype(
	derive(
		Debug,
		Default,
		PartialEq,
		Eq,
		PartialOrd,
		Ord,
		Clone,
		Copy,
		AsRef,
		Deref,
		Borrow,
		Display,
		From,
		Into,
		Deserialize,
		Serialize
	),
	derive_unsafe(Format, MaxSize),
	default = 0,
	const_fn
)]
pub struct MilliAmp(u16);

#[nutype(
	derive(
		Debug,
		PartialEq,
		Eq,
		PartialOrd,
		Ord,
		Clone,
		Copy,
		AsRef,
		Deref,
		Borrow,
		Display,
		Default,
		From,
		Into,
		Deserialize,
		Serialize
	),
	derive_unsafe(Format, MaxSize),
	default = 0,
	const_fn
)]
pub struct MilliVolt(u16);

#[derive(Debug, Default, PartialEq, Eq, MaxSize, Format, Clone, Copy, Serialize, Deserialize)]
pub struct BiCommand {
	pub load: LoadState,
	pub reset: Reset,
	pub clear_fault: ClearFault,
	pub allow_undercurrent: AllowUndercurrent,
}

#[derive(Debug, Default, PartialEq, Eq, MaxSize, Format, Clone, Copy, Serialize, Deserialize)]
pub enum ClearFault {
	#[default]
	No,
	Yes,
}

#[derive(Debug, Default, PartialEq, Eq, MaxSize, Format, Clone, Copy, Serialize, Deserialize)]
/// Enter the reset state
pub enum Reset {
	#[default]
	No,
	/// User must disconnect and reconncect the battery
	Yes,
}

#[derive(Debug, Default, PartialEq, Eq, MaxSize, Format, Clone, Copy, Serialize, Deserialize)]
pub enum AllowUndercurrent {
	#[default]
	No,
	Yes,
}

#[derive(Debug, Default, PartialEq, Eq, MaxSize, Format, Clone, Copy, Serialize, Deserialize)]
pub enum LoadState {
	#[default]
	Off,
	On,
}

#[derive(Debug, PartialEq, Eq, MaxSize, Format, Clone, Copy, Deserialize, Serialize)]
pub struct Measurement {
	pub vbat: MilliVolt,
	pub ibat: MilliAmp,
	pub t: u64,
}

#[derive(Debug, PartialEq, Eq, MaxSize, Format, Clone, Copy, Deserialize, Serialize)]
pub struct BIReply {
	pub measurement: Option<Measurement>,
	pub fault: Result<(), Fault>,
}

#[derive(Debug, PartialEq, Eq, MaxSize, Format, Clone, Copy, Deserialize, Serialize)]
pub struct Fault {
	pub kind: FaultKind,
	pub time: u64,
}

#[derive(Debug, PartialEq, Eq, MaxSize, Format, Clone, Copy, Deserialize, Serialize)]
pub enum FaultKind {
	/// Some I2C fault
	I2C(I2CError), // TODO: specific codes
	/// Heater is set to on but current draw is low
	Undercurrent,
	/// Battery not detected,
	NoBattery,
	Overcurrent,
}

#[derive(Debug, PartialEq, Eq, MaxSize, Format, Clone, Copy, Deserialize, Serialize)]
pub enum I2CError {
	InaVinCurrent(TiwmError),
	InaVinVoltage(TiwmError),
	InaVinConfig(TiwmError),
	InaVinId(TiwmError),
}

#[derive(Debug, PartialEq, Eq, MaxSize, Format, Clone, Copy, Deserialize, Serialize)]
pub enum TiwmError {
	TxBufferTooLong,
	RxBufferTooLong,
	Transmit,
	Receive,
	RAMBufferTooSmall,
	AddressNack,
	DataNack,
	Overrun,
	Timeout,
	Unknown,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_max_reply_size() {
		assert!(REPLY_MAX_SIZE <= u8::MAX as usize);
	}

	#[test]
	fn test_max_command_size() {
		assert!(COMMAND_MAX_SIZE <= u8::MAX as usize);
	}
}
