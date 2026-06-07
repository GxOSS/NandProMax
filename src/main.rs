mod interface;
mod ftdi;
mod picoflasher;

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;

use crate::interface::cli::{Cli, Command};
use crate::picoflasher::pfc::{
	Client, CMD_EMMC_DETECT, CMD_EMMC_GET_EXT_CSD, CMD_EMMC_INIT, CMD_EMMC_READ, CMD_EMMC_READ_STREAM,
	CMD_EMMC_WRITE, CMD_EMMC_WRITE_MULTI, CMD_GET_FLASH_CONFIG, CMD_GET_VERSION, CMD_READ_FLASH,
	CMD_READ_FLASH_STREAM, CMD_SET_SMC_WORKAROUND, CMD_START_SMC, CMD_STOP_SMC, CMD_WRITE_FLASH,
	CMD_WRITE_FLASH_MULTI, EMMC_BLOCK_BYTES, NAND_BLOCK_BYTES,
};

fn main() -> Result<()> {
	let cli = Cli::parse();
	let timeout = Duration::from_millis(cli.timeout_ms);
	match cli.command {
		Command::FtdiList => {
			ftdi_list()?;
			println!("ok");
		}
		Command::FtdiReadNand {
			out,
			start,
			count,
			ftdi_desc,
			ftdi_index,
			freq_hz,
		} => {
			let elapsed = ftdi_read_nand(out, start, count, &ftdi_desc, ftdi_index, freq_hz)?;
			println!("ok ({:.3}s)", elapsed.as_secs_f64());
		}
		Command::FtdiWriteNand {
			input,
			start,
			count,
			ftdi_desc,
			ftdi_index,
			freq_hz,
		} => {
			let elapsed = ftdi_write_nand(input, start, count, &ftdi_desc, ftdi_index, freq_hz)?;
			println!("ok ({:.3}s)", elapsed.as_secs_f64());
		}
		Command::ReadNand { out, start, count } => {
			let (mut client, resolved) = if let Some(port) = &cli.serial {
				Client::connect_usb(port, timeout).with_context(|| format!("failed to open serial {port}"))?
			} else {
				Client::connect_tcp(&cli.addr, timeout)
					.with_context(|| format!("failed to connect to {}", cli.addr))?
			};
			eprintln!("connected to {resolved}");

			let (flash_config, blocks_total) = prepare_nand(&mut client)?;
			let blocks = count.unwrap_or(blocks_total.saturating_sub(start));
			eprintln!("flash_config=0x{flash_config:08x} blocks={blocks} start={start}");
			read_nand(&mut client, out, start, blocks)?;
			println!("ok");
		}
		Command::WriteNand { input, start } => {
			let (mut client, resolved) = if let Some(port) = &cli.serial {
				Client::connect_usb(port, timeout).with_context(|| format!("failed to open serial {port}"))?
			} else {
				Client::connect_tcp(&cli.addr, timeout)
					.with_context(|| format!("failed to connect to {}", cli.addr))?
			};
			eprintln!("connected to {resolved}");

			let (flash_config, blocks_total) = prepare_nand(&mut client)?;
			eprintln!("flash_config=0x{flash_config:08x} start={start} max_blocks={blocks_total}");
			write_nand(&mut client, input, start)?;
			println!("ok");
		}
		Command::ReadEmmc { out, start, count } => {
			let (mut client, resolved) = if let Some(port) = &cli.serial {
				Client::connect_usb(port, timeout).with_context(|| format!("failed to open serial {port}"))?
			} else {
				Client::connect_tcp(&cli.addr, timeout)
					.with_context(|| format!("failed to connect to {}", cli.addr))?
			};
			eprintln!("connected to {resolved}");

			let blocks_total = prepare_emmc(&mut client)?;
			let blocks = count.unwrap_or(blocks_total.saturating_sub(start));
			eprintln!("emmc_blocks={blocks} start={start}");
			read_emmc(&mut client, out, start, blocks)?;
			println!("ok");
		}
		Command::WriteEmmc { input, start } => {
			let (mut client, resolved) = if let Some(port) = &cli.serial {
				Client::connect_usb(port, timeout).with_context(|| format!("failed to open serial {port}"))?
			} else {
				Client::connect_tcp(&cli.addr, timeout)
					.with_context(|| format!("failed to connect to {}", cli.addr))?
			};
			eprintln!("connected to {resolved}");

			let blocks_total = prepare_emmc(&mut client)?;
			eprintln!("start={start} max_blocks={blocks_total}");
			write_emmc(&mut client, input, start)?;
			println!("ok");
		}
	}

	Ok(())
}

fn ftdi_read_nand(
	out: std::path::PathBuf,
	start: u32,
	count: Option<u32>,
	ftdi_desc: &str,
	ftdi_index: Option<i32>,
	freq_hz: u32,
) -> Result<Duration> {
	use crate::ftdi::spi::{sfc_init, xnand_clear_status, xnand_read_page_raw, XSpi};

	eprintln!("ftdi freq_hz={freq_hz}");
	let mut xspi = XSpi::open(ftdi_desc, ftdi_index, freq_hz)?;
	xspi.enter_flash_mode()?;

	let flash_config = xspi.read_u32(0x00)?;
	let geom = sfc_init(flash_config)?;
	let total_pages = geom.pages_count_in_nand;
	let pages = count.unwrap_or(total_pages.saturating_sub(start));

	eprintln!(
		"flash_config=0x{flash_config:08x} nand={}MB start={} pages={}",
		geom.nand_size_mb, start, pages
	);

	let f = File::create(out).context("open output")?;
	let mut f = BufWriter::with_capacity(1024 * 1024, f);

	let t0 = Instant::now();
	let mut page_buf = [0u8; 0x210];
	for i in 0..pages {
		if (i & 0xFF) == 0 {
			xnand_clear_status(&mut xspi).context("clear status")?;
		}
		let page = start + i;
		xnand_read_page_raw(&mut xspi, page, &mut page_buf)
			.with_context(|| format!("read page {page}"))?;
		f.write_all(&page_buf)?;

		if (i & 0xFF) == 0 {
			eprintln!("read {}/{} pages", i + 1, pages);
		}
	}

	f.flush().context("flush output")?;
	Ok(t0.elapsed())
}

fn ftdi_write_nand(
	input: std::path::PathBuf,
	start: u32,
	count: Option<u32>,
	ftdi_desc: &str,
	ftdi_index: Option<i32>,
	freq_hz: u32,
) -> Result<Duration> {
	use crate::ftdi::spi::{sfc_init, xnand_clear_status, xnand_write_page_raw, XSpi};

	let input_meta = std::fs::metadata(&input).context("stat input")?;
	let input_len = input_meta.len() as usize;
	if input_len % 0x210 != 0 {
		bail!("input size must be a multiple of 0x210 bytes (raw page size)");
	}

	let file_pages = (input_len / 0x210) as u32;
	let pages = count.unwrap_or(file_pages);
	if pages > file_pages {
		bail!("input has {file_pages} pages but --count={pages} requested");
	}

	eprintln!("ftdi freq_hz={freq_hz}");
	let mut xspi = XSpi::open(ftdi_desc, ftdi_index, freq_hz)?;
	xspi.enter_flash_mode()?;

	let flash_config = xspi.read_u32(0x00)?;
	let geom = sfc_init(flash_config)?;
	let total_pages = geom.pages_count_in_nand;
	if start >= total_pages {
		bail!("start page {start} out of range (total pages {total_pages})");
	}
	if start + pages > total_pages {
		bail!("requested range {}..{} out of range (total pages {total_pages})", start, start + pages);
	}

	eprintln!(
		"flash_config=0x{flash_config:08x} nand={}MB start={} pages={} (input_pages={file_pages})",
		geom.nand_size_mb, start, pages
	);

	let f = File::open(input).context("open input")?;
	let mut f = BufReader::with_capacity(1024 * 1024, f);

	let t0 = Instant::now();
	let mut page_buf = [0u8; 0x210];
	for i in 0..pages {
		if (i & 0xFF) == 0 {
			xnand_clear_status(&mut xspi).context("clear status")?;
		}

		f.read_exact(&mut page_buf).context("read input page")?;
		let page = start + i;
		xnand_write_page_raw(&mut xspi, page, &page_buf)
			.with_context(|| format!("write page {page}"))?;

		if (i & 0xFF) == 0 {
			eprintln!("wrote {}/{} pages", i + 1, pages);
		}
	}

	Ok(t0.elapsed())
}

fn ftdi_list() -> Result<()> {
	use ftdi_embedded_hal::libftd2xx;

	let n = libftd2xx::num_devices().context("FT_ListDevices(NUMBER_ONLY)")?;
	eprintln!("libftd2xx num_devices={n}");
	let devs = libftd2xx::list_devices().context("FT_GetDeviceInfoList")?;
	eprintln!("libftd2xx list_devices len={}", devs.len());
	for (i, d) in devs.iter().enumerate() {
		eprintln!(
			"[{i}] vid=0x{:04x} pid=0x{:04x} type={:?} open={} serial={:?} desc={:?}",
			d.vendor_id, d.product_id, d.device_type, d.port_open, d.serial_number, d.description
		);
	}

	if let Ok(devs) = libftd2xx::list_devices_fs() {
		eprintln!("libftd2xx list_devices_fs len={}", devs.len());
		for (i, d) in devs.iter().enumerate() {
			eprintln!(
				"[fs {i}] vid=0x{:04x} pid=0x{:04x} type={:?} open={} serial={:?} desc={:?}",
				d.vendor_id, d.product_id, d.device_type, d.port_open, d.serial_number, d.description
			);
		}
	}
	Ok(())
}

fn prepare_nand(client: &mut Client) -> Result<(u32, u32)> {
	let _ver = client.cmd_u32(CMD_GET_VERSION, 0)?;
	let _ = client.cmd_u32(CMD_SET_SMC_WORKAROUND, 0)?;
	let _ = client.cmd_u32(CMD_STOP_SMC, 0)?;
	std::thread::sleep(Duration::from_millis(500));

	let flash_config = client.cmd_u32(CMD_GET_FLASH_CONFIG, 0)?;
	if flash_config == 0 || flash_config == 0xFFFF_FFFF {
		bail!("console not found (flash_config=0x{flash_config:08x})");
	}

	let flash_size_bytes = flash_size_from_config(flash_config)
		.ok_or_else(|| anyhow::anyhow!("unknown flash size for flash_config=0x{flash_config:08x}"))?;
	let blocks = (flash_size_bytes / 512) as u32;
	Ok((flash_config, blocks))
}

fn flash_size_from_config(flash_config: u32) -> Option<usize> {
	let major = (flash_config >> 17) & 3;
	let minor = (flash_config >> 4) & 3;

	let size_mb = if major >= 1 {
		match minor {
			0 => {
				if major != 1 {
					16
				} else {
					return None;
				}
			}
			1 => {
				if major != 1 {
					64
				} else {
					16
				}
			}
			2 | 3 => {
				let a = (flash_config >> 19) & 0x3;
				let b = (flash_config >> 21) & 0xF;
				8usize.checked_shl((a + b) as u32)?
			}
			_ => return None,
		}
	} else {
		8usize.checked_shl(minor as u32)?
	};

	Some(size_mb * 1024 * 1024)
}

fn read_nand(client: &mut Client, out: std::path::PathBuf, start: u32, count: u32) -> Result<()> {
	let f = File::create(out).context("open output")?;
	let mut f = BufWriter::with_capacity(1024 * 1024, f);

	if start == 0 {
		client.start_stream(CMD_READ_FLASH_STREAM, count)?;
		for i in 0..count {
			let (ret, data) = client.recv_stream_block(NAND_BLOCK_BYTES)?;
			if ret != 0 {
				bail!("read failed at block {i}: 0x{ret:08x}");
			}
			f.write_all(&data.unwrap()).context("write output")?;

			if (i & 0xFF) == 0 {
				eprintln!("read {}/{} blocks", i + 1, count);
			}
		}
	} else {
		for i in 0..count {
			let lba = start + i;
			let (ret, data) = client.read_with_ret(CMD_READ_FLASH, lba, NAND_BLOCK_BYTES)?;
			if ret != 0 {
				bail!("read failed at lba {lba}: 0x{ret:08x}");
			}
			f.write_all(&data.unwrap()).context("write output")?;

			if (i & 0xFF) == 0 {
				eprintln!("read {}/{} blocks", i + 1, count);
			}
		}
	}

	Ok(())
}

fn write_nand(client: &mut Client, input: std::path::PathBuf, start: u32) -> Result<()> {
	let mut buf = vec![];
	File::open(input)
		.context("open input")?
		.read_to_end(&mut buf)
		.context("read input")?;

	if buf.len() % NAND_BLOCK_BYTES != 0 {
		bail!("input size must be a multiple of 0x210 (got 0x{:x})", buf.len());
	}

	let blocks = (buf.len() / NAND_BLOCK_BYTES) as u32;
	let mut i = 0u32;
	if client.supports_multi_write() {
		while i < blocks {
			let remaining = blocks - i;
			let chunk_blocks = remaining.min(64);
			let lba = start + i;

			let off = (i as usize) * NAND_BLOCK_BYTES;
			let end = off + (chunk_blocks as usize) * NAND_BLOCK_BYTES;
			let (ret, idx) = client.write_multi(CMD_WRITE_FLASH_MULTI, lba, NAND_BLOCK_BYTES, &buf[off..end])?;
			if ret != 0 {
				bail!("write failed at lba {}: 0x{ret:08x}", lba + idx);
			}

			i += chunk_blocks;
			eprintln!("written {}/{} blocks", i, blocks);
		}
	} else {
		while i < blocks {
			let lba = start + i;
			let off = (i as usize) * NAND_BLOCK_BYTES;
			let end = off + NAND_BLOCK_BYTES;
			let ret = client.write_single(CMD_WRITE_FLASH, lba, &buf[off..end])?;
			if ret != 0 {
				bail!("write failed at lba {}: 0x{ret:08x}", lba);
			}
			i += 1;
			if (i & 0xFF) == 0 || i == blocks {
				eprintln!("written {}/{} blocks", i, blocks);
			}
		}
	}

	Ok(())
}

fn prepare_emmc(client: &mut Client) -> Result<u32> {
	let _ver = client.cmd_u32(CMD_GET_VERSION, 0)?;
	let _ = client.cmd_u32(CMD_SET_SMC_WORKAROUND, 0)?;
	let _ = client.cmd_u32(CMD_STOP_SMC, 0)?;
	std::thread::sleep(Duration::from_millis(500));

	let detect = client.cmd_u8(CMD_EMMC_DETECT, 0)?;
	if detect == 0 {
		bail!("eMMC not detected");
	}

	let ret = client.cmd_u32(CMD_EMMC_INIT, 0)?;
	if ret != 0 {
		bail!("EMMC_INIT failed: {ret}");
	}

	let ext = client.cmd_exact_bytes(CMD_EMMC_GET_EXT_CSD, 0, 512)?;

	let sec_count = u32::from_le_bytes(ext[212..216].try_into().unwrap());
	if sec_count == 0 {
		bail!("invalid EXT_CSD SEC_COUNT=0");
	}
	Ok(sec_count)
}

fn read_emmc(client: &mut Client, out: std::path::PathBuf, start: u32, count: u32) -> Result<()> {
	let f = File::create(out).context("open output")?;
	let mut f = BufWriter::with_capacity(1024 * 1024, f);

	if start == 0 {
		client.start_stream(CMD_EMMC_READ_STREAM, count)?;
		for i in 0..count {
			let (ret, data) = client.recv_stream_block(EMMC_BLOCK_BYTES)?;
			if ret != 0 {
				bail!("read failed at block {i}: {ret}");
			}
			f.write_all(&data.unwrap()).context("write output")?;

			if (i & 0xFF) == 0 {
				eprintln!("read {}/{} blocks", i + 1, count);
			}
		}
	} else {
		for i in 0..count {
			let lba = start + i;
			let (ret, data) = client.read_with_ret(CMD_EMMC_READ, lba, EMMC_BLOCK_BYTES)?;
			if ret != 0 {
				bail!("read failed at lba {lba}: {ret}");
			}
			f.write_all(&data.unwrap()).context("write output")?;

			if (i & 0xFF) == 0 {
				eprintln!("read {}/{} blocks", i + 1, count);
			}
		}
	}

	Ok(())
}

fn write_emmc(client: &mut Client, input: std::path::PathBuf, start: u32) -> Result<()> {
	let mut buf = vec![];
	File::open(input)
		.context("open input")?
		.read_to_end(&mut buf)
		.context("read input")?;

	if buf.len() % EMMC_BLOCK_BYTES != 0 {
		bail!("input size must be a multiple of 0x200 (got 0x{:x})", buf.len());
	}

	let blocks = (buf.len() / EMMC_BLOCK_BYTES) as u32;
	let mut i = 0u32;
	if client.supports_multi_write() {
		while i < blocks {
			let remaining = blocks - i;
			let chunk_blocks = remaining.min(64);
			let lba = start + i;

			let off = (i as usize) * EMMC_BLOCK_BYTES;
			let end = off + (chunk_blocks as usize) * EMMC_BLOCK_BYTES;
			let (ret, idx) = client.write_multi(CMD_EMMC_WRITE_MULTI, lba, EMMC_BLOCK_BYTES, &buf[off..end])?;
			if ret != 0 {
				bail!("write failed at lba {}: {ret}", lba + idx);
			}

			i += chunk_blocks;
			eprintln!("written {}/{} blocks", i, blocks);
		}
	} else {
		while i < blocks {
			let lba = start + i;
			let off = (i as usize) * EMMC_BLOCK_BYTES;
			let end = off + EMMC_BLOCK_BYTES;
			let ret = client.write_single(CMD_EMMC_WRITE, lba, &buf[off..end])?;
			if ret != 0 {
				bail!("write failed at lba {}: {ret}", lba);
			}
			i += 1;
			if (i & 0x3FF) == 0 || i == blocks {
				eprintln!("written {}/{} blocks", i, blocks);
			}
		}
	}

	let _ = client.cmd_u32(CMD_START_SMC, 0);

	Ok(())
}
