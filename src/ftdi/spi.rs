use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use ftdi_embedded_hal as hal;
use hal::eh1::spi::Polarity;
use hal::eh1::spi::SpiBus as _;
use hal::libftd2xx;
use hal::ftdi_mpsse::MpsseSettings;
use hal::ftdi_mpsse::{ClockData, ClockDataIn, ClockDataOut, MpsseCmdExecutor};

use crate::ftdi::gpio::{Device, XboxPins};

pub struct XSpi {
	hal: hal::FtHal<Device>,
	spi: hal::Spi<Device>,
	pins: XboxPins,
	bit_reverse: bool,
	idle_high: bool,
}

impl XSpi {
	pub fn open(desc: &str, ftdi_index: Option<i32>, freq_hz: u32) -> Result<Self> {
		if let Some(index) = ftdi_index {
			let device = Device::with_index(index)?;
			return Self::open_one(device, freq_hz).with_context(|| format!("ftdi-index={index}"));
		}

		if desc != "auto" {
			let device = open_ftdi2232h(desc, None)?;
			return Self::open_one(device, freq_hz);
		}

		if let Ok(device) = open_ftdi2232h_auto() {
			if let Ok(xspi) = Self::open_one(device, freq_hz) {
				return Ok(xspi);
			}
		}

		let num = libftd2xx::num_devices().context("query number of FTDI devices")? as i32;
		if num <= 0 {
			bail!("libftd2xx sees 0 devices");
		}

		let mut errs: Vec<String> = vec![];
		for index in 0..num {
			match Device::with_index(index) {
				Ok(device) => match Self::open_one(device, freq_hz) {
					Ok(xspi) => return Ok(xspi),
					Err(e) => errs.push(format!("[{index}] {e:#}")),
				},
				Err(e) => errs.push(format!("[{index}] open failed: {e:#}")),
			}
		}

		Err(anyhow!(
			"unable to open/probe any FTDI interface by index\n{}",
			errs.join("\n")
		))
	}

	fn open_one(device: Device, freq_hz: u32) -> Result<Self> {
		let settings = MpsseSettings {
			clock_frequency: Some(freq_hz),
			latency_timer: Duration::from_millis(1),
			in_transfer_size: 65536,
			read_timeout: Duration::from_secs(5),
			write_timeout: Duration::from_secs(5),
			..Default::default()
		};
		let hal = hal::FtHal::init(device, &settings).context("init MPSSE")?;
		let spi = hal.spi().context("init SPI bus")?;
		let pins = XboxPins::new(&hal).context("init GPIO")?;

		let mut xspi = Self {
			hal,
			spi,
			pins,
			bit_reverse: false,
			idle_high: false,
		};

		xspi.spi
			.set_clock_polarity(Polarity::IdleLow)
			.context("set SPI CPOL")?;

		let mut tried: Vec<(Polarity, bool, u32)> = vec![];
		for pol in [Polarity::IdleLow, Polarity::IdleHigh] {
			xspi.spi
				.set_clock_polarity(pol)
				.context("set SPI CPOL")?;
			xspi.idle_high = matches!(pol, Polarity::IdleHigh);

			xspi.enter_flash_mode().context("enter flash mode")?;
			for bit_reverse in [false, true] {
				xspi.bit_reverse = bit_reverse;
				let cfg = xspi.read_u32(0x00).context("read flash_config")?;
				let cfg2 = xspi.read_u32(0x00).context("read flash_config (2)")?;
				let cfg = if cfg != 0 { cfg } else { cfg2 };
				tried.push((pol, bit_reverse, cfg));
				if is_plausible_flash_config(cfg) {
					return Ok(xspi);
				}
			}
		}

		let mut msg = String::new();
		for (pol, br, cfg) in tried {
			msg.push_str(&format!("CPOL={pol:?} bitrev={br} flash_config=0x{cfg:08x}\n"));
		}
		Err(anyhow!("unable to read plausible flash_config\n{msg}"))
	}

	pub fn enter_flash_mode(&mut self) -> Result<()> {
		self.pins.set_gpio(false, true)?;
		self.pins.set_cs(true)?;
		std::thread::sleep(Duration::from_millis(35));

		self.pins.set_gpio(false, false)?;
		self.pins.set_cs(false)?;
		std::thread::sleep(Duration::from_millis(35));

		self.pins.set_gpio(true, true)?;
		std::thread::sleep(Duration::from_millis(35));

		Ok(())
	}

	pub fn read_u32(&mut self, reg: u8) -> Result<u32> {
		let mut buf = [0xFFu8; 6];
		buf[0] = (reg << 2) | 1;
		buf[1] = 0xFF;

		self.cs_asserted_transfer_in_place(&mut buf)?;

		Ok(u32::from_le_bytes(buf[2..6].try_into().unwrap()))
	}

	#[allow(dead_code)]
	pub fn read_u16(&mut self, reg: u8) -> Result<u16> {
		let mut buf = [0xFFu8; 4];
		buf[0] = (reg << 2) | 1;
		buf[1] = 0xFF;

		self.cs_asserted_transfer_in_place(&mut buf)?;

		Ok(u16::from_le_bytes(buf[2..4].try_into().unwrap()))
	}

	pub fn read_u8(&mut self, reg: u8) -> Result<u8> {
		let mut buf = [0xFFu8; 3];
		buf[0] = (reg << 2) | 1;
		buf[1] = 0xFF;

		self.cs_asserted_transfer_in_place(&mut buf)?;

		Ok(buf[2])
	}

	pub fn write_u32(&mut self, reg: u8, val: u32) -> Result<()> {
		let mut buf = [0u8; 5];
		buf[0] = (reg << 2) | 2;
		buf[1..5].copy_from_slice(&val.to_le_bytes());

		self.cs_asserted_write(&buf)?;
		Ok(())
	}

	fn cs_asserted_write(&mut self, write: &[u8]) -> Result<()> {
		self.pins.set_cs(false)?;
		let mut w = write.to_vec();
		if self.bit_reverse {
			for b in &mut w {
				*b = b.reverse_bits();
			}
		}
		self.spi.write(&w)?;
		self.pins.set_cs(true)?;
		Ok(())
	}

	fn cs_asserted_transfer_in_place(&mut self, buf: &mut [u8]) -> Result<()> {
		self.pins.set_cs(false)?;
		if self.bit_reverse {
			for b in buf.iter_mut() {
				*b = b.reverse_bits();
			}
		}
		self.spi.transfer_in_place(buf)?;
		if self.bit_reverse {
			for b in buf.iter_mut() {
				*b = b.reverse_bits();
			}
		}
		self.pins.set_cs(true)?;
		Ok(())
	}

	fn mpsse_gpio_consts(&self) -> (u8, u8, u8) {
		let dir: u8 = 0x3B;
		let base = (if self.idle_high { 0x01 } else { 0x00 }) | 0x08 | 0x10 | 0x20;
		let cs_lo = base & !0x08;
		let cs_hi = base | 0x08;
		(dir, cs_lo, cs_hi)
	}

	fn mpsse_clock_modes(&self) -> (ClockDataOut, ClockDataIn, ClockData) {
		if self.idle_high {
			(ClockDataOut::MsbPos, ClockDataIn::MsbNeg, ClockData::MsbNegIn)
		} else {
			(ClockDataOut::MsbNeg, ClockDataIn::MsbPos, ClockData::MsbPosIn)
		}
	}

	fn cmd_set_gpio_lower(cmd: &mut Vec<u8>, value: u8, dir: u8) {
		cmd.extend_from_slice(&[0x80, value, dir]);
	}

	fn cmd_send_immediate(cmd: &mut Vec<u8>) {
		cmd.push(0x87);
	}

	fn cmd_clock_data_out(cmd: &mut Vec<u8>, mode: ClockDataOut, data: &[u8]) {
		let len = data.len() - 1;
		let op: u8 = mode.into();
		cmd.extend_from_slice(&[op, (len & 0xFF) as u8, ((len >> 8) & 0xFF) as u8]);
		cmd.extend_from_slice(data);
	}

	fn cmd_clock_data_in(cmd: &mut Vec<u8>, mode: ClockDataIn, len: usize) {
		let len = len - 1;
		let op: u8 = mode.into();
		cmd.extend_from_slice(&[op, (len & 0xFF) as u8, ((len >> 8) & 0xFF) as u8]);
	}

	fn cmd_clock_data(cmd: &mut Vec<u8>, mode: ClockData, data: &[u8]) {
		let len = data.len() - 1;
		let op: u8 = mode.into();
		cmd.extend_from_slice(&[op, (len & 0xFF) as u8, ((len >> 8) & 0xFF) as u8]);
		cmd.extend_from_slice(data);
	}

	fn append_write_u32_cmd(&self, cmd: &mut Vec<u8>, reg: u8, val: u32) {
		let (dir, cs_lo, cs_hi) = self.mpsse_gpio_consts();
		let (clk_out, _, _) = self.mpsse_clock_modes();

		let mut buf = [0u8; 5];
		buf[0] = (reg << 2) | 2;
		buf[1..5].copy_from_slice(&val.to_le_bytes());
		if self.bit_reverse {
			for b in &mut buf {
				*b = b.reverse_bits();
			}
		}

		Self::cmd_set_gpio_lower(cmd, cs_lo, dir);
		Self::cmd_clock_data_out(cmd, clk_out, &buf);
		Self::cmd_set_gpio_lower(cmd, cs_hi, dir);
	}

	fn append_read_data_words_fast_cmd(&self, cmd: &mut Vec<u8>, words: usize) -> Result<()> {
		let expected = words
			.checked_mul(4)
			.ok_or_else(|| anyhow!("word count too large"))?;
		let _ = expected;

		let (dir, cs_lo, cs_hi) = self.mpsse_gpio_consts();
		let (clk_out, clk_in, _) = self.mpsse_clock_modes();

		let mut w0 = [(0x08u8 << 2) | 2, 0, 0, 0, 0];
		let mut r0 = [(0x10u8 << 2) | 1, 0xFF];
		if self.bit_reverse {
			for b in &mut w0 {
				*b = b.reverse_bits();
			}
			for b in &mut r0 {
				*b = b.reverse_bits();
			}
		}

		let per_word_bytes = 3 + (3 + w0.len()) + 3 + 3 + (3 + r0.len()) + 3 + 3;
		cmd.reserve(words * per_word_bytes);
		for _ in 0..words {
			Self::cmd_set_gpio_lower(cmd, cs_lo, dir);
			Self::cmd_clock_data_out(cmd, clk_out, &w0);
			Self::cmd_set_gpio_lower(cmd, cs_hi, dir);
			Self::cmd_set_gpio_lower(cmd, cs_lo, dir);
			Self::cmd_clock_data_out(cmd, clk_out, &r0);
			Self::cmd_clock_data_in(cmd, clk_in, 4);
			Self::cmd_set_gpio_lower(cmd, cs_hi, dir);
		}
		Ok(())
	}

	fn xnand_wait_ready_fast(&mut self, mut timeout: u32) -> Result<()> {
		let (dir, cs_lo, cs_hi) = self.mpsse_gpio_consts();
		let (_, _, clk) = self.mpsse_clock_modes();
		let mut tx = [(0x04u8 << 2) | 1, 0xFF, 0xFF];
		if self.bit_reverse {
			for b in &mut tx {
				*b = b.reverse_bits();
			}
		}

		const BATCH: usize = 256;
		let mut rx = vec![0u8; BATCH * tx.len()];
		let per_poll_bytes = 3 + (3 + tx.len()) + 3;
		let mut cmd = Vec::with_capacity(BATCH * per_poll_bytes + 1);
		while timeout > 0 {
			let n = (timeout as usize).min(BATCH);
			cmd.clear();
			for _ in 0..n {
				Self::cmd_set_gpio_lower(&mut cmd, cs_lo, dir);
				Self::cmd_clock_data(&mut cmd, clk, &tx);
				Self::cmd_set_gpio_lower(&mut cmd, cs_hi, dir);
			}
			Self::cmd_send_immediate(&mut cmd);

			self.hal.with_device(|d| -> Result<()> {
				d.send(&cmd).context("mpsse send")?;
				d.recv(&mut rx[..n * tx.len()]).context("mpsse recv")?;
				Ok(())
			})?;

			for i in 0..n {
				let mut st = rx[i * tx.len() + 2];
				if self.bit_reverse {
					st = st.reverse_bits();
				}
				if (st & 0x01) == 0 {
					return Ok(());
				}
			}

			timeout = timeout.saturating_sub(n as u32);
		}

		bail!("timeout waiting for NAND ready");
	}

	fn xnand_start_page_read(&mut self, page: u32) -> Result<()> {
		let mut cmd = Vec::with_capacity(128);
		self.append_write_u32_cmd(&mut cmd, 0x0C, page << 9);
		self.append_write_u32_cmd(&mut cmd, 0x08, 0x03);
		Self::cmd_send_immediate(&mut cmd);

		self.hal.with_device(|d| -> Result<()> {
			d.send(&cmd).context("mpsse send")?;
			Ok(())
		})?;

		Ok(())
	}

	fn xnand_finish_page_read(&mut self, out: &mut [u8; 0x210]) -> Result<()> {
		let mut cmd = Vec::with_capacity(16384);
		self.append_write_u32_cmd(&mut cmd, 0x0C, 0);
		self.append_read_data_words_fast_cmd(&mut cmd, 0x84)?;
		Self::cmd_send_immediate(&mut cmd);

		self.hal.with_device(|d| -> Result<()> {
			d.send(&cmd).context("mpsse send")?;
			d.recv(out).context("mpsse recv")?;
			Ok(())
		})?;

		if self.bit_reverse {
			for b in out {
				*b = b.reverse_bits();
			}
		}

		Ok(())
	}

	fn xnand_load_page_buffer(&mut self, data: &[u8; 0x210]) -> Result<()> {
		let mut cmd = Vec::with_capacity(8192);
		self.append_write_u32_cmd(&mut cmd, 0x0C, 0);

		for chunk in data.chunks_exact(4) {
			let w = u32::from_le_bytes(chunk.try_into().unwrap());
			self.append_write_u32_cmd(&mut cmd, 0x10, w);
			self.append_write_u32_cmd(&mut cmd, 0x08, 0x01);
		}

		Self::cmd_send_immediate(&mut cmd);
		self.hal.with_device(|d| -> Result<()> {
			d.send(&cmd).context("mpsse send")?;
			Ok(())
		})?;
		Ok(())
	}

	fn xnand_write_execute(&mut self, page: u32) -> Result<()> {
		let mut cmd = Vec::with_capacity(256);
		self.append_write_u32_cmd(&mut cmd, 0x0C, page << 9);
		self.append_write_u32_cmd(&mut cmd, 0x08, 0x55);
		self.append_write_u32_cmd(&mut cmd, 0x08, 0xAA);
		self.append_write_u32_cmd(&mut cmd, 0x08, 0x04);
		Self::cmd_send_immediate(&mut cmd);

		self.hal.with_device(|d| -> Result<()> {
			d.send(&cmd).context("mpsse send")?;
			Ok(())
		})?;

		self.xnand_wait_ready_fast(0x1000).context("wait ready")?;
		Ok(())
	}
}

fn open_ftdi2232h(desc_hint: &str, ftdi_index: Option<i32>) -> Result<Device> {
	if let Some(index) = ftdi_index {
		return Device::with_index(index);
	}

	if desc_hint == "auto" {
		return open_ftdi2232h_auto();
	}

	if let Ok(dev) = Device::with_description(desc_hint) {
		return Ok(dev);
	}

	let devices = libftd2xx::list_devices().context("list ftdi devices")?;
	let hint = desc_hint.trim().to_lowercase();

	let mut matches: Vec<libftd2xx::DeviceInfo> = devices
		.into_iter()
		.filter(|d| {
			let ddesc = d.description.to_lowercase();
			ddesc.contains(&hint) || hint.contains(&ddesc)
		})
		.collect();

	if matches.is_empty() {
		bail!(
			"no FTDI devices match description hint: {desc_hint:?}. Try --ftdi-desc auto, or pass one of the exact descriptions shown by `libftd2xx::list_devices()`"
		);
	}

	matches.sort_by_key(|d| score_desc(&d.description));
	let chosen = matches.first().unwrap();

	Device::with_description(&chosen.description)
		.with_context(|| format!("open FTDI by matched description: {:?}", chosen.description))
}

fn open_ftdi2232h_auto() -> Result<Device> {
	let num = libftd2xx::num_devices().context("query number of FTDI devices")?;
	let devices = libftd2xx::list_devices().context("list ftdi devices")?;
	let mut cands: Vec<libftd2xx::DeviceInfo> = devices
		.into_iter()
		.filter(|d| d.vendor_id == 0x0403 && d.product_id == 0x6010)
		.collect();

	if cands.is_empty() {
		if num == 0 {
			return Err(anyhow!(
				"libftd2xx sees 0 devices. On Linux this usually means the kernel VCP driver still owns the interface(s) (ftdi_sio/usbserial) or permissions are blocking access."
			));
		}

		return Err(anyhow!(
			"libftd2xx sees {num} device(s) but none match 0403:6010. Run `cargo run -- ftdi-list` to see what libftd2xx is reporting."
		));
	}

	cands.sort_by_key(|d| score_desc(&d.description));
	let chosen = cands.first().unwrap();

	Device::with_description(&chosen.description)
		.with_context(|| format!("open FTDI by auto-selected description: {:?}", chosen.description))
}

fn score_desc(desc: &str) -> (u8, String) {
	let d = desc.to_lowercase();
	let prefer_b = if d.ends_with(" b") || d.contains(" b ") {
		0
	} else {
		1
	};
	let prefer_a = if d.ends_with(" a") || d.contains(" a ") {
		0
	} else {
		1
	};
	(prefer_b, format!("{prefer_a}:{d}"))
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SfcGeometry {
	pub large_block: u32,
	pub page_size: u32,
	pub page_size_raw: u32,
	pub meta_size: u32,
	pub page_count_in_block: u32,
	pub block_size: u32,
	pub block_size_raw: u32,
	pub nand_size_data_partition_bytes: u32,
	pub nand_size_bytes: u32,
	pub nand_size_mb: u32,
	pub pages_count_in_nand: u32,
}

pub fn sfc_init(flash_config: u32) -> Result<SfcGeometry> {
	let page_size = 0x200u32;
	let meta_size = 0x10u32;
	let page_size_raw = page_size + meta_size;

	let (large_block, block_size, nand_size_data_partition_bytes, nand_size_bytes) =
		match (flash_config >> 17) & 0x03 {
			0 => {
				let block_size = 0x4000;
				let nand_size_data_partition_bytes = match (flash_config >> 4) & 0x03 {
					0 => bail!("unsupported 8MB flash_config=0x{flash_config:08x}"),
					1 => 0x400u32 << 0xE,
					2 => 0x800u32 << 0xE,
					3 => 0x1000u32 << 0xE,
					_ => unreachable!(),
				};
				(0u32, block_size, nand_size_data_partition_bytes, nand_size_data_partition_bytes)
			}
			1 | 2 => match (flash_config >> 4) & 0x03 {
				0 => {
					if ((flash_config >> 17) & 0x03) == 0x01 {
						bail!("unsupported flash_config=0x{flash_config:08x}")
					} else {
						let block_size = 0x4000;
						let nand_size_data_partition_bytes = 0x400u32 << 0xE;
						(0u32, block_size, nand_size_data_partition_bytes, nand_size_data_partition_bytes)
					}
				}
				1 => {
					let block_size = 0x4000;
					let nand_size_data_partition_bytes = if ((flash_config >> 17) & 0x03) == 0x01 {
						0x400u32 << 0xE
					} else {
						0x1000u32 << 0xE
					};
					(0u32, block_size, nand_size_data_partition_bytes, nand_size_data_partition_bytes)
				}
				2 => {
					let block_size = 0x20000;
					let a = (flash_config >> 19) & 0x3;
					let b = (flash_config >> 21) & 0xF;
					let shift = a + b + 0x17;
					let nand_size_bytes = 1u32.checked_shl(shift).ok_or_else(|| {
						anyhow!("invalid NAND size shift for flash_config=0x{flash_config:08x}")
					})?;
					let nand_size_data_partition_bytes = 0x4000000;
					(2u32, block_size, nand_size_data_partition_bytes, nand_size_bytes)
				}
				3 => {
					let block_size = 0x40000;
					let a = (flash_config >> 19) & 0x3;
					let b = (flash_config >> 21) & 0xF;
					let shift = a + b + 0x17;
					let nand_size_bytes = 1u32.checked_shl(shift).ok_or_else(|| {
						anyhow!("invalid NAND size shift for flash_config=0x{flash_config:08x}")
					})?;
					let nand_size_data_partition_bytes = 0x8000000;
					(3u32, block_size, nand_size_data_partition_bytes, nand_size_bytes)
				}
				_ => unreachable!(),
			},
			_ => bail!("unknown southbridge for flash_config=0x{flash_config:08x}"),
		};

	let page_count_in_block = block_size / page_size;
	let block_size_raw = page_count_in_block * page_size_raw;
	let pages_count_in_nand = nand_size_data_partition_bytes / page_size;
	let nand_size_mb = nand_size_data_partition_bytes >> 20;

	Ok(SfcGeometry {
		large_block,
		page_size,
		page_size_raw,
		meta_size,
		page_count_in_block,
		block_size,
		block_size_raw,
		nand_size_data_partition_bytes,
		nand_size_bytes,
		nand_size_mb,
		pages_count_in_nand,
	})
}

pub fn xnand_read_page_raw(xspi: &mut XSpi, page: u32, out: &mut [u8; 0x210]) -> Result<()> {
	xspi.xnand_start_page_read(page)?;
	xspi.xnand_wait_ready_fast(0x1000).context("wait ready")?;
	xspi.xnand_finish_page_read(out)?;
	Ok(())
}

pub fn xnand_clear_status(xspi: &mut XSpi) -> Result<()> {
	let st = xspi.read_u32(0x04)?;
	xspi.write_u32(0x04, st)?;
	Ok(())
}

pub fn xnand_write_page_raw(xspi: &mut XSpi, page: u32, data: &[u8; 0x210]) -> Result<()> {
	xspi.xnand_load_page_buffer(data)?;
	xspi.xnand_write_execute(page)?;
	Ok(())
}

fn is_plausible_flash_config(v: u32) -> bool {
	if v == 0 || v == 0xFFFF_FFFF {
		return false;
	}
	let b = v.to_le_bytes();
	if b[0] == b[1] && b[1] == b[2] && b[2] == b[3] {
		return false;
	}
	let major = (v >> 17) & 3;
	let minor = (v >> 4) & 3;
	major <= 2 && minor <= 3
}
