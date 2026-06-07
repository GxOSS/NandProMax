use anyhow::{Context, Result};
use ftdi_embedded_hal as hal;
use hal::eh1::digital::OutputPin as _;
use hal::libftd2xx;
use hal::libftd2xx::{FtdiCommon as _, FtdiMpsse as _};
use hal::ftdi_mpsse::{MpsseCmdExecutor, MpsseSettings};

pub struct Device {
	inner: libftd2xx::Ftdi,
}

impl Device {
	pub fn with_description(desc: &str) -> Result<Self> {
		let inner = libftd2xx::Ftdi::with_description(desc)
			.with_context(|| format!("open device by description: {desc:?}"))?;
		Ok(Self { inner })
	}

	pub fn with_index(index: i32) -> Result<Self> {
		let inner = libftd2xx::Ftdi::with_index(index).with_context(|| format!("open device index {index}"))?;
		Ok(Self { inner })
	}
}

impl libftd2xx::FtdiCommon for Device {
	const DEVICE_TYPE: libftd2xx::DeviceType = libftd2xx::DeviceType::FT2232H;

	fn handle(&mut self) -> *mut std::ffi::c_void {
		self.inner.handle()
	}
}

impl libftd2xx::FtdiMpsse for Device {}

impl MpsseCmdExecutor for Device {
	type Error = libftd2xx::TimeoutError;

	fn init(&mut self, settings: &MpsseSettings) -> Result<(), Self::Error> {
		self.initialize_mpsse(settings)
	}

	fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
		self.write_all(data)
	}

	fn recv(&mut self, data: &mut [u8]) -> Result<(), Self::Error> {
		self.read_all(data)
	}
}

pub struct XboxPins {
	pub cs: hal::OutputPin<Device>,
	pub xx: hal::OutputPin<Device>,
	pub ej: hal::OutputPin<Device>,
}

impl XboxPins {
	pub fn new(hal: &hal::FtHal<Device>) -> Result<Self> {
		let mut cs = hal.ad3()?;
		let mut xx = hal.ad4()?;
		let mut ej = hal.ad5()?;

		cs.set_high()?;
		xx.set_low()?;
		ej.set_low()?;

		Ok(Self { cs, xx, ej })
	}

	pub fn set_cs(&mut self, high: bool) -> Result<()> {
		if high {
			self.cs.set_high()?;
		} else {
			self.cs.set_low()?;
		}
		Ok(())
	}

	pub fn set_gpio(&mut self, xx: bool, ej: bool) -> Result<()> {
		if xx {
			self.xx.set_high()?;
		} else {
			self.xx.set_low()?;
		}

		if ej {
			self.ej.set_high()?;
		} else {
			self.ej.set_low()?;
		}

		Ok(())
	}
}
