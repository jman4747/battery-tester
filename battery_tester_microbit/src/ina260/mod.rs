use battery_tester_common::{MilliAmp, MilliVolt};
use embassy_nrf::twim;

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, defmt::Format)]
pub enum Register {
	// Configuration Register
	CONFIG = 0x00,
	// Contains the value of the current flowing through the shunt resistor
	CURRENT = 0x01,
	// Bus voltage measurement data
	VOLTAGE = 0x02,
	// Contains the value of the calculated power being delivered to the load
	POWER = 0x03,
	// Alert configuration and conversion ready flag
	MASK_ENABLE = 0x06,
	// Contains the limit value to compare to the selected alert function
	ALERT_LIMIT = 0x07,
	// Contains unique manufacturer identification number
	MANUFACTURER_ID = 0xFE,
	// Contains unique die identification number
	DIE_ID = 0xFF,
}

impl Register {
	#[inline(always)]
	pub fn addr(self) -> u8 {
		self as u8
	}
}

impl From<Register> for u8 {
	fn from(r: Register) -> u8 {
		r as u8
	}
}

#[allow(dead_code)]
#[derive(Copy, Clone, defmt::Format)]
/// Averaging Mode
/// Determines the number of samples that are collected and averaged.
pub enum Averaging {
	// No averaging (default)
	AVG1 = 0b0000_0000_0000_0000,
	// 4 times averaging
	AVG4 = 0b0000_0010_0000_0000,
	// 16 times averaging
	AVG16 = 0b0000_0100_0000_0000,
	// 64 times averaging
	AVG64 = 0b0000_0110_0000_0000,
	// 128 times averaging
	AVG128 = 0b0000_1000_0000_0000,
	// 256 times averaging
	AVG256 = 0b0000_1010_0000_0000,
	// 512 times averaging
	AVG512 = 0b0000_1100_0000_0000,
	// 1024 times averaging
	AVG1024 = 0b0000_1110_0000_0000,
}

impl Averaging {
	#[inline(always)]
	pub fn bits(self) -> u16 {
		self as u16
	}
}

#[allow(dead_code)]
#[derive(Copy, Clone, defmt::Format)]
/// Bus Voltage Conversion Time
/// Sets the conversion time for the bus voltage measurement
pub enum BVConvTime {
	// Conversion time = 140 µs
	US140 = 0b0000_0000_0000_0000,
	// Conversion time = 204 µs
	US204 = 0b0000_0000_0100_0000,
	// Conversion time = 332 µs
	US332 = 0b0000_0000_1000_0000,
	// Conversion time = 588 µs
	US588 = 0b0000_0000_1100_0000,
	// Conversion time = 1.1 ms (default)
	MS1_1 = 0b0000_0001_0000_0000,
	// Conversion time = 2.116 ms
	MS2_116 = 0b0000_0001_0100_0000,
	// Conversion time = 4.156 ms
	MS4_156 = 0b0000_0001_1000_0000,
	// Conversion time = 8.244 ms
	MS8_244 = 0b0000_0001_1100_0000,
}

impl BVConvTime {
	#[inline(always)]
	pub fn bits(self) -> u16 {
		self as u16
	}
}

#[allow(dead_code)]
#[derive(Copy, Clone, defmt::Format)]
/// Shunt Current Conversion Time
/// Sets the conversion time for the shunt current measurement
pub enum SCConvTime {
	// Conversion time = 140 µs
	US140 = 0b0000_0000_0000_0000,
	// Conversion time = 204 µs
	US204 = 0b0000_0000_0000_1000,
	// Conversion time = 332 µs
	US332 = 0b0000_0000_0001_0000,
	// Conversion time = 588 µs
	US588 = 0b0000_0000_0001_1000,
	// Conversion time = 1.1 ms (default)
	MS1_1 = 0b0000_0000_0010_0000,
	// Conversion time = 2.116 ms
	MS2_116 = 0b0000_0000_0010_1000,
	// Conversion time = 4.156 ms
	MS4_156 = 0b0000_0000_0011_0000,
	// Conversion time = 8.244 ms
	MS8_244 = 0b0000_0000_0011_1000,
}

impl SCConvTime {
	#[inline(always)]
	pub fn bits(self) -> u16 {
		self as u16
	}
}

#[allow(dead_code)]
#[derive(Copy, Clone, defmt::Format)]
/// Operating Mode
/// Selects continuous, triggered, or power-down mode of operation.
pub enum OperMode {
	// Power-Down (or Shutdown)
	SHUTDOWN = 0b0000_0000_0000_0000,
	// = Shunt Current, Triggered
	SCT = 0b0000_0000_0000_0001,
	// = Shunt Current, Triggered
	BVT = 0b0000_0000_0000_0010,
	// = Shunt Current + Bus Voltage, Triggered
	SCBVT = 0b0000_0000_0000_0011,
	// = Shunt Current, Continuous
	SCC = 0b0000_0000_0000_0101,
	// = Bus Voltage, Continuous
	BVC = 0b0000_0000_0000_0110,
	// = Shunt Current + Bus Voltage, Continuous (default)
	SCBVC = 0b0000_0000_0000_0111,
}

impl OperMode {
	#[inline(always)]
	pub fn bits(self) -> u16 {
		self as u16
	}
}

#[allow(dead_code)]
#[derive(Copy, Clone)]
/// Mask/Enable Register
///
/// The Mask/Enable Register selects the function that is enabled to control the ALERT pin as well as how that pin
/// functions. If multiple functions are enabled, the highest significant bit position Alert Function (D15-D11) takes
/// priority and responds to the Alert Limit Register.
pub enum MaskEnable {
	/// Over Current Limit
	///
	/// Setting this bit high configures the ALERT pin to be asserted if the current
	/// measurement following a conversion exceeds the value programmed in the Alert
	/// Limit Register.
	OCL = 0b1000_0000_0000_0000,
	/// Under Current Limit
	///
	/// Setting this bit high configures the ALERT pin to be asserted if the current
	/// measurement following a conversion drops below the value programmed in the
	/// Alert Limit Register.
	UCL = 0b0100_0000_0000_0000,
	/// Bus Voltage Over-Voltage
	///
	/// Setting this bit high configures the ALERT pin to be asserted if the bus voltage
	/// measurement following a conversion exceeds the value programmed in the Alert
	/// Limit Register.
	BOL = 0b0010_0000_0000_0000,
	/// Bus Voltage Under-Voltage
	///
	/// Setting this bit high configures the ALERT pin to be asserted if the bus voltage
	/// measurement following a conversion drops below the value programmed in the
	/// Alert Limit Register.
	BUL = 0b0001_0000_0000_0000,
	/// Power Over-Limit
	///
	/// Setting this bit high configures the ALERT pin to be asserted if the Power
	/// calculation made following a bus voltage measurement exceeds the value
	/// programmed in the Alert Limit Register.
	POL = 0b0000_1000_0000_0000,
	/// Conversion Ready
	///
	/// Setting this bit high configures the ALERT pin to be asserted when the Conversion
	/// Ready Flag, Bit 3, is asserted indicating that the device is ready for the next
	/// conversion.
	CNVR = 0b0000_0100_0000_0000,
	/// Alert Function Flag
	///
	/// While only one Alert Function can be monitored at the ALERT pin at a time, the
	/// Conversion Ready can also be enabled to assert the ALERT pin. Reading the Alert
	/// Function Flag following an alert allows the user to determine if the Alert Function
	/// was the source of the Alert.
	///
	/// When the Alert Latch Enable bit is set to Latch mode, the Alert Function Flag bit
	/// clears only when the Mask/Enable Register is read. When the Alert Latch Enable
	/// bit is set to Transparent mode, the Alert Function Flag bit is cleared following the
	/// next conversion that does not result in an Alert condition.
	AFF = 0b0000_0000_0001_0000,
	/// Conversion Ready
	///
	/// Although the device can be read at any time, and the data from the last conversion
	/// is available, the Conversion Ready Flag bit is provided to help coordinate one-shot
	/// or triggered conversions. The Conversion Ready Flag bit is set after all
	/// conversions, averaging, and multiplications are complete. Conversion Ready Flag
	/// bit clears under the following conditions:
	///
	/// 1.) Writing to the Configuration Register (except for Power-Down selection)
	/// 2.) Reading the Mask/Enable Register
	CVRF = 0b0000_0000_0000_1000,
	/// Math Overflow Flag
	///
	/// This bit is set to '1' if an arithmetic operation resulted in an overflow error. It
	/// indicates that power data may have exceeded the maximum reportable value of
	/// 419.43 W.
	OVF = 0b0000_0000_0000_0100,
	/// Alert Polarity bit
	///
	/// 1 = Inverted (active-high open collector)
	/// 0 = Normal (active-low open collector) (default)
	APOL = 0b0000_0000_0000_0010,
	/// Alert Latch Enable; configures the latching feature of the ALERT pin and Alert Flag
	/// bits.
	///
	/// 1 = Latch enabled
	/// 0 = Transparent (default)
	///
	/// When the Alert Latch Enable bit is set to Transparent mode, the ALERT pin and
	/// Flag bit resets to the idle states when the fault has been cleared. When the Alert
	/// Latch Enable bit is set to Latch mode, the ALERT pin and Alert Flag bit remains
	/// active following a fault until the Mask/Enable Register has been read.
	LEN = 0b0000_0000_0000_0001,
}

impl MaskEnable {
	#[inline(always)]
	pub fn bits(self) -> u16 {
		self as u16
	}
}

#[derive(Copy, Clone)]
pub struct INA260Config {
	om: OperMode,
	am: Averaging,
	scct: SCConvTime,
	bvct: BVConvTime,
}

impl Default for INA260Config {
	fn default() -> Self {
		Self::new()
	}
}

impl INA260Config {
	pub fn new() -> Self {
		Self {
			om: OperMode::SCBVC,
			am: Averaging::AVG4,
			scct: SCConvTime::MS1_1,
			bvct: BVConvTime::MS1_1,
		}
	}
	pub fn set_operating_mode(&mut self, om: OperMode) -> &mut Self {
		self.om = om;
		self
	}
	pub fn set_averaging_mode(&mut self, am: Averaging) -> &mut Self {
		self.am = am;
		self
	}
	pub fn set_sccov_time(&mut self, scct: SCConvTime) -> &mut Self {
		self.scct = scct;
		self
	}
	pub fn set_bvcov_time(&mut self, bvct: BVConvTime) -> &mut Self {
		self.bvct = bvct;
		self
	}
	pub fn as_be_bytes(&self) -> [u8; 2] {
		let as_u16 = self.om.bits() | self.am.bits() | self.scct.bits() | self.bvct.bits();
		as_u16.to_be_bytes()
	}
}

pub async fn set_config(
	address: u8,
	i2c: &mut twim::Twim<'static>,
	conf: INA260Config,
) -> Result<(), twim::Error> {
	let bytes = conf.as_be_bytes();
	i2c.write(address, &[Register::CONFIG.into(), bytes[0], bytes[1]])
		.await
}

pub async fn shutdown<I>(address: u8, i2c: &mut twim::Twim<'static>) -> Result<(), twim::Error>
where
	I: embassy_nrf::twim::Instance,
{
	let bytes = OperMode::SHUTDOWN.bits().to_be_bytes();
	i2c.write(address, &[Register::CONFIG.into(), bytes[0], bytes[1]])
		.await
}

/// Returns current in milliamps
pub async fn get_amps(address: u8, i2c: &mut twim::Twim<'static>) -> Result<MilliAmp, twim::Error> {
	let mut buffer = [0u8; 2];
	let raw = i32::from({
		i2c.write_read(address, &[Register::CURRENT.addr()], &mut buffer)
			.await?;
		u16::from_be_bytes(buffer) as i16
	});
	Ok(MilliAmp::new((raw * 1250 / 1000).unsigned_abs() as u16))
}

/// Returns voltage as millivolts
pub async fn get_voltage(
	address: u8,
	i2c: &mut twim::Twim<'static>,
) -> Result<MilliVolt, twim::Error> {
	let mut buffer = [0u8; 2];
	let raw = u32::from({
		i2c.write_read(address, &[Register::VOLTAGE.addr()], &mut buffer)
			.await?;
		u16::from_be_bytes(buffer)
	});
	Ok(MilliVolt::new((raw * 1250 / 1000) as u16))
}
