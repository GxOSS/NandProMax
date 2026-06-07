use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

const PFC_MAGIC: u32 = 0x5046_4331;
const PFC_VERSION: u16 = 1;

const PFC_MSG_REQUEST: u16 = 0;
const PFC_MSG_RESPONSE: u16 = 1;

#[derive(Debug, Clone)]
pub struct Frame {
	pub msg_type: u16,
	pub payload: Vec<u8>,
}

pub struct Client {
	reader: BufReader<TcpStream>,
	writer: BufWriter<TcpStream>,
}

impl Client {
	pub fn connect<A: ToSocketAddrs>(addr: A, timeout: Duration) -> Result<(Self, SocketAddr)> {
		let mut last_err: Option<anyhow::Error> = None;
		let mut resolved: Option<SocketAddr> = None;

		for sock in addr.to_socket_addrs().context("failed to resolve address")? {
			resolved = Some(sock);
			match TcpStream::connect_timeout(&sock, timeout) {
				Ok(stream) => {
					stream
						.set_read_timeout(Some(timeout))
						.context("failed to set read timeout")?;
					stream
						.set_write_timeout(Some(timeout))
						.context("failed to set write timeout")?;
					stream
						.set_nodelay(true)
						.context("failed to set nodelay")?;
					let reader = BufReader::with_capacity(64 * 1024, stream.try_clone().context("clone stream for reader")?);
					let writer = BufWriter::with_capacity(64 * 1024, stream);
					return Ok((Self { reader, writer }, sock));
				}
				Err(e) => last_err = Some(anyhow!(e).context(format!("connect to {sock} failed"))),
			}
		}

		match (resolved, last_err) {
			(Some(_), Some(e)) => Err(e),
			_ => bail!("no socket addresses found"),
		}
	}

	pub fn send_request(&mut self, payload: &[u8]) -> Result<()> {
		self.send_frame(PFC_MSG_REQUEST, payload)
	}

	pub fn recv_response(&mut self) -> Result<Frame> {
		let frame = self.recv_frame()?;
		if frame.msg_type != PFC_MSG_RESPONSE {
			bail!("unexpected message type {}, expected response", frame.msg_type);
		}
		Ok(frame)
	}

	pub fn request_response(&mut self, payload: &[u8]) -> Result<Frame> {
		self.send_request(payload)?;
		self.recv_response()
	}

	#[allow(dead_code)]
	pub fn send_cmd(&mut self, cmd: u8, lba: u32, extra: &[u8]) -> Result<()> {
		let mut payload = Vec::with_capacity(5 + extra.len());
		payload.push(cmd);
		payload.extend_from_slice(&lba.to_le_bytes());
		payload.extend_from_slice(extra);
		self.send_request(&payload)
	}

	pub fn cmd_u32(&mut self, cmd: u8, lba: u32) -> Result<u32> {
		let frame = self.request_response(&cmd_payload(cmd, lba))?;
		if frame.payload.len() != 4 {
			bail!("expected 4-byte response, got {}", frame.payload.len());
		}
		Ok(u32::from_le_bytes(frame.payload[..4].try_into().unwrap()))
	}

	#[allow(dead_code)]
	pub fn cmd_bytes(&mut self, cmd: u8, lba: u32) -> Result<Vec<u8>> {
		let frame = self.request_response(&cmd_payload(cmd, lba))?;
		Ok(frame.payload)
	}

	fn send_frame(&mut self, msg_type: u16, payload: &[u8]) -> Result<()> {
		let mut hdr = [0u8; 12];
		hdr[0..4].copy_from_slice(&PFC_MAGIC.to_le_bytes());
		hdr[4..6].copy_from_slice(&PFC_VERSION.to_le_bytes());
		hdr[6..8].copy_from_slice(&msg_type.to_le_bytes());
		hdr[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());

		self.writer.write_all(&hdr).context("tcp write header failed")?;
		if !payload.is_empty() {
			self.writer
				.write_all(payload)
				.context("tcp write payload failed")?;
		}
		self.writer.flush().context("tcp flush failed")?;
		Ok(())
	}

	fn recv_frame(&mut self) -> Result<Frame> {
		let mut hdr = [0u8; 12];
		self.reader.read_exact(&mut hdr).context("tcp read header failed")?;

		let magic = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
		let version = u16::from_le_bytes(hdr[4..6].try_into().unwrap());
		let msg_type = u16::from_le_bytes(hdr[6..8].try_into().unwrap());
		let len = u32::from_le_bytes(hdr[8..12].try_into().unwrap()) as usize;

		if magic != PFC_MAGIC {
			bail!("bad magic 0x{magic:08x}");
		}
		if version != PFC_VERSION {
			bail!("unsupported version {version}");
		}

		let mut payload = vec![0u8; len];
		if len != 0 {
			self.reader
				.read_exact(&mut payload)
				.context("tcp read payload failed")?;
		}

		Ok(Frame { msg_type, payload })
	}
}

pub fn cmd_payload(cmd: u8, lba: u32) -> [u8; 5] {
	let mut buf = [0u8; 5];
	buf[0] = cmd;
	buf[1..5].copy_from_slice(&lba.to_le_bytes());
	buf
}
