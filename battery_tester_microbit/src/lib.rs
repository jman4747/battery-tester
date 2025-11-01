#![no_std]

use battery_tester_common::{MilliAmp, MilliVolt, TiwmError};
use embassy_nrf::twim;
use embassy_time::{Instant, Timer};

pub mod ina260;
pub mod pwm;

/// How long to wait to ensure battery connection is secure
pub const BAT_CONNECT_DEBOUNCE_MS: u64 = 250;

#[derive(Copy, Clone, Default)]
pub struct EmbassyDelayer;

impl EmbassyDelayer {
	pub const fn new() -> Self {
		Self {}
	}
}

impl embedded_hal_async::delay::DelayNs for EmbassyDelayer {
	async fn delay_ns(&mut self, ns: u32) {
		Timer::after_nanos(ns as u64).await
	}

	async fn delay_us(&mut self, us: u32) {
		Timer::after_micros(us as u64).await
	}

	async fn delay_ms(&mut self, ms: u32) {
		Timer::after_millis(ms as u64).await
	}
}

pub const fn twim_err_to_common(twim_err: twim::Error) -> TiwmError {
	match twim_err {
		twim::Error::TxBufferTooLong => TiwmError::TxBufferTooLong,
		twim::Error::RxBufferTooLong => TiwmError::RxBufferTooLong,
		twim::Error::Transmit => TiwmError::Transmit,
		twim::Error::Receive => TiwmError::Receive,
		twim::Error::RAMBufferTooSmall => TiwmError::RAMBufferTooSmall,
		twim::Error::AddressNack => TiwmError::AddressNack,
		twim::Error::DataNack => TiwmError::DataNack,
		twim::Error::Overrun => TiwmError::Overrun,
		twim::Error::Timeout => TiwmError::Timeout,
		_ => TiwmError::Unknown,
	}
}

fn milliamp_to_u32(milliamp: &MilliAmp) -> u32 {
	u16::from(*milliamp) as u32
}

fn millivolt_to_u32(millivolt: &MilliVolt) -> u32 {
	u16::from(*millivolt) as u32
}

pub struct DaqDataQueue {
	index: usize,
	start: u64,
	milliamps: [MilliAmp; 10],
	millivolts: [MilliVolt; 10],
}

impl DaqDataQueue {
	pub fn reset(&mut self) {
		self.index = 0;
		self.start = 0;
		self.milliamps = [MilliAmp::default(); 10];
		self.millivolts = [MilliVolt::default(); 10];
	}

	pub const fn default_const() -> Self {
		Self {
			index: 0,
			start: 0,
			milliamps: [MilliAmp::new(0u16); 10],
			millivolts: [MilliVolt::new(0u16); 10],
		}
	}

	pub fn avg_milliamps(&self) -> MilliAmp {
		let sum: u32 = self.milliamps.iter().map(milliamp_to_u32).sum();
		MilliAmp::new((sum / 10) as u16)
	}

	pub fn avg_millivolts(&self) -> MilliVolt {
		let sum: u32 = self.millivolts.iter().map(millivolt_to_u32).sum();
		MilliVolt::new((sum / 10) as u16)
	}

	pub fn get_latest_vin_v(&self) -> MilliVolt {
		self.millivolts[self.index]
	}

	pub fn get_latest_vin_amps(&self) -> MilliAmp {
		self.milliamps[self.index]
	}

	pub fn push(
		&mut self,
		vin_milliamps: MilliAmp,
		vin_millivolts: MilliVolt,
	) -> Option<(MilliVolt, MilliAmp, u64)> {
		self.milliamps[self.index] = vin_milliamps;
		self.millivolts[self.index] = vin_millivolts;
		if self.index == 9 {
			self.index = 0;
			Some((
				self.avg_millivolts(),
				self.avg_milliamps(),
				(Instant::now().as_millis() + self.start) / 2,
			))
		} else if self.index == 0 {
			self.start = Instant::now().as_millis();
			self.index += 1;
			None
		} else {
			self.index += 1;
			None
		}
	}
}
