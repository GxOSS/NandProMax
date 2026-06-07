use std::time::Duration;

use anyhow::{bail, Result};

use crate::picoflasher::{pftcp, usb};

pub const CMD_GET_VERSION: u8 = 0x00;
pub const CMD_GET_FLASH_CONFIG: u8 = 0x01;
pub const CMD_READ_FLASH: u8 = 0x02;
pub const CMD_WRITE_FLASH: u8 = 0x03;
pub const CMD_READ_FLASH_STREAM: u8 = 0x04;
#[allow(dead_code)]
pub const CMD_ERASE_FLASH: u8 = 0x05;
pub const CMD_WRITE_FLASH_MULTI: u8 = 0x06;

pub const CMD_SET_SMC_WORKAROUND: u8 = 0x20;
pub const CMD_STOP_SMC: u8 = 0x21;
pub const CMD_START_SMC: u8 = 0x22;

pub const CMD_EMMC_DETECT: u8 = 0x50;
pub const CMD_EMMC_INIT: u8 = 0x51;
#[allow(dead_code)]
pub const CMD_EMMC_GET_CID: u8 = 0x52;
#[allow(dead_code)]
pub const CMD_EMMC_GET_CSD: u8 = 0x53;
pub const CMD_EMMC_GET_EXT_CSD: u8 = 0x54;
pub const CMD_EMMC_READ: u8 = 0x55;
pub const CMD_EMMC_READ_STREAM: u8 = 0x56;
pub const CMD_EMMC_WRITE: u8 = 0x57;
pub const CMD_EMMC_WRITE_MULTI: u8 = 0x58;

pub const NAND_BLOCK_BYTES: usize = 0x210;
pub const EMMC_BLOCK_BYTES: usize = 0x200;

pub enum Client {
	Tcp(pftcp::Client),
	Usb(usb::Client),
}

impl Client {
	pub fn connect_tcp(addr: &str, timeout: Duration) -> Result<(Self, String)> {
		let (c, resolved) = pftcp::Client::connect(addr, timeout)?;
		Ok((Self::Tcp(c), resolved.to_string()))
	}

	pub fn connect_usb(port: &str, timeout: Duration) -> Result<(Self, String)> {
		let c = usb::Client::open(port, timeout)?;
		Ok((Self::Usb(c), port.to_string()))
	}

	pub fn supports_multi_write(&self) -> bool {
		matches!(self, Self::Tcp(_))
	}

	pub fn cmd_u32(&mut self, cmd: u8, lba: u32) -> Result<u32> {
		match self {
			Self::Tcp(c) => c.cmd_u32(cmd, lba),
			Self::Usb(c) => c.cmd_u32(cmd, lba),
		}
	}

	pub fn cmd_u8(&mut self, cmd: u8, lba: u32) -> Result<u8> {
		match self {
			Self::Tcp(c) => {
				let frame = c.request_response(&pftcp::cmd_payload(cmd, lba))?;
				if frame.payload.len() != 1 {
					bail!("expected 1-byte response, got {}", frame.payload.len());
				}
				Ok(frame.payload[0])
			}
			Self::Usb(c) => c.cmd_u8(cmd, lba),
		}
	}

	pub fn cmd_exact_bytes(&mut self, cmd: u8, lba: u32, len: usize) -> Result<Vec<u8>> {
		match self {
			Self::Tcp(c) => {
				let frame = c.request_response(&pftcp::cmd_payload(cmd, lba))?;
				if frame.payload.len() != len {
					bail!("expected {}-byte response, got {}", len, frame.payload.len());
				}
				Ok(frame.payload)
			}
			Self::Usb(c) => c.cmd_exact_bytes(cmd, lba, len),
		}
	}

	pub fn read_with_ret(&mut self, cmd: u8, lba: u32, data_len: usize) -> Result<(u32, Option<Vec<u8>>)> {
		match self {
			Self::Tcp(c) => {
				let frame = c.request_response(&pftcp::cmd_payload(cmd, lba))?;
				if frame.payload.len() < 4 {
					bail!("short response ({} bytes)", frame.payload.len());
				}
				let ret = u32::from_le_bytes(frame.payload[0..4].try_into().unwrap());
				if ret != 0 {
					return Ok((ret, None));
				}
				if frame.payload.len() != 4 + data_len {
					bail!("expected {}-byte payload, got {}", 4 + data_len, frame.payload.len());
				}
				Ok((ret, Some(frame.payload[4..].to_vec())))
			}
			Self::Usb(c) => c.read_with_ret(cmd, lba, data_len),
		}
	}

	pub fn start_stream(&mut self, cmd: u8, count: u32) -> Result<()> {
		match self {
			Self::Tcp(c) => c.send_request(&pftcp::cmd_payload(cmd, count)),
			Self::Usb(c) => c.send_cmd(cmd, count, &[]),
		}
	}

	pub fn recv_stream_block(&mut self, data_len: usize) -> Result<(u32, Option<Vec<u8>>)> {
		match self {
			Self::Tcp(c) => {
				let frame = c.recv_response()?;
				if frame.payload.len() < 4 {
					bail!("short response ({} bytes)", frame.payload.len());
				}
				let ret = u32::from_le_bytes(frame.payload[0..4].try_into().unwrap());
				if ret != 0 {
					return Ok((ret, None));
				}
				if frame.payload.len() != 4 + data_len {
					bail!("expected {}-byte payload, got {}", 4 + data_len, frame.payload.len());
				}
				Ok((ret, Some(frame.payload[4..].to_vec())))
			}
			Self::Usb(c) => c.recv_stream_block(data_len),
		}
	}

	pub fn write_multi(&mut self, cmd: u8, lba: u32, block_bytes: usize, data: &[u8]) -> Result<(u32, u32)> {
		let count = (data.len() / block_bytes) as u16;
		if data.len() != (count as usize) * block_bytes {
			bail!("data length must be a multiple of block size");
		}

		match self {
			Self::Tcp(c) => {
				let mut payload = Vec::with_capacity(5 + 2 + data.len());
				payload.push(cmd);
				payload.extend_from_slice(&lba.to_le_bytes());
				payload.extend_from_slice(&count.to_le_bytes());
				payload.extend_from_slice(data);
				c.send_request(&payload)?;
				let frame = c.recv_response()?;
				if frame.payload.len() != 8 {
					bail!("expected 8-byte response, got {}", frame.payload.len());
				}
				let ret = u32::from_le_bytes(frame.payload[0..4].try_into().unwrap());
				let idx = u32::from_le_bytes(frame.payload[4..8].try_into().unwrap());
				Ok((ret, idx))
			}
			Self::Usb(_) => bail!("multi-write not supported over USB serial"),
		}
	}

	pub fn write_single(&mut self, cmd: u8, lba: u32, data: &[u8]) -> Result<u32> {
		match self {
			Self::Tcp(c) => {
				let mut payload = Vec::with_capacity(5 + data.len());
				payload.push(cmd);
				payload.extend_from_slice(&lba.to_le_bytes());
				payload.extend_from_slice(data);
				let frame = c.request_response(&payload)?;
				if frame.payload.len() != 4 {
					bail!("expected 4-byte response, got {}", frame.payload.len());
				}
				Ok(u32::from_le_bytes(frame.payload[0..4].try_into().unwrap()))
			}
			Self::Usb(c) => c.write_single(cmd, lba, data),
		}
	}
}

pub fn cmd_payload(cmd: u8, lba: u32) -> [u8; 5] {
	let mut buf = [0u8; 5];
	buf[0] = cmd;
	buf[1..5].copy_from_slice(&lba.to_le_bytes());
	buf
}
