use std::io::{Read, Write};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::picoflasher::pfc::cmd_payload;

pub struct Client {
	port: Box<dyn serialport::SerialPort>,
}

impl Client {
	pub fn open(path: &str, timeout: Duration) -> Result<Self> {
		let port = serialport::new(path, 115_200)
			.timeout(timeout)
			.open()
			.with_context(|| format!("open serial port {path}"))?;
		Ok(Self { port })
	}

	pub fn send_cmd(&mut self, cmd: u8, lba: u32, extra: &[u8]) -> Result<()> {
		let mut payload = Vec::with_capacity(5 + extra.len());
		payload.extend_from_slice(&cmd_payload(cmd, lba));
		payload.extend_from_slice(extra);
		self.port.write_all(&payload).context("serial write")?;
		Ok(())
	}

	pub fn cmd_u32(&mut self, cmd: u8, lba: u32) -> Result<u32> {
		self.send_cmd(cmd, lba, &[])?;
		self.read_u32()
	}

	pub fn cmd_u8(&mut self, cmd: u8, lba: u32) -> Result<u8> {
		self.send_cmd(cmd, lba, &[])?;
		let mut b = [0u8; 1];
		self.port.read_exact(&mut b).context("serial read u8")?;
		Ok(b[0])
	}

	pub fn cmd_exact_bytes(&mut self, cmd: u8, lba: u32, len: usize) -> Result<Vec<u8>> {
		self.send_cmd(cmd, lba, &[])?;
		let mut buf = vec![0u8; len];
		self.port
			.read_exact(&mut buf)
			.with_context(|| format!("serial read {len} bytes"))?;
		Ok(buf)
	}

	pub fn read_with_ret(&mut self, cmd: u8, lba: u32, data_len: usize) -> Result<(u32, Option<Vec<u8>>)> {
		self.send_cmd(cmd, lba, &[])?;
		let ret = self.read_u32()?;
		if ret != 0 {
			return Ok((ret, None));
		}
		let mut data = vec![0u8; data_len];
		self.port
			.read_exact(&mut data)
			.with_context(|| format!("serial read {data_len} bytes"))?;
		Ok((ret, Some(data)))
	}

	pub fn recv_stream_block(&mut self, data_len: usize) -> Result<(u32, Option<Vec<u8>>)> {
		let ret = self.read_u32()?;
		if ret != 0 {
			return Ok((ret, None));
		}
		let mut data = vec![0u8; data_len];
		self.port
			.read_exact(&mut data)
			.with_context(|| format!("serial read {data_len} bytes"))?;
		Ok((ret, Some(data)))
	}

	pub fn write_single(&mut self, cmd: u8, lba: u32, data: &[u8]) -> Result<u32> {
		self.send_cmd(cmd, lba, data)?;
		self.read_u32()
	}

	fn read_u32(&mut self) -> Result<u32> {
		let mut buf = [0u8; 4];
		self.port.read_exact(&mut buf).context("serial read u32")?;
		Ok(u32::from_le_bytes(buf))
	}
}
