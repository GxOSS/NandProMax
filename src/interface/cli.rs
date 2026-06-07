use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "picoclient")]
#[command(about = "PicoFlasher client (TCP or USB serial)", long_about = None)]
pub struct Cli {
	#[arg(long = "ip", alias = "addr", default_value = "192.168.4.1:3232")]
	pub addr: String,

	#[arg(long)]
	pub serial: Option<String>,

	#[arg(long, default_value = "3000")]
	pub timeout_ms: u64,

	#[command(subcommand)]
	pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
	ReadNand {
		#[arg(long)]
		out: PathBuf,

		#[arg(long, default_value_t = 0)]
		start: u32,

		#[arg(long)]
		count: Option<u32>,
	},

	WriteNand {
		#[arg(long)]
		input: PathBuf,

		#[arg(long, default_value_t = 0)]
		start: u32,
	},

	ReadEmmc {
		#[arg(long)]
		out: PathBuf,

		#[arg(long, default_value_t = 0)]
		start: u32,

		#[arg(long)]
		count: Option<u32>,
	},

	WriteEmmc {
		#[arg(long)]
		input: PathBuf,

		#[arg(long, default_value_t = 0)]
		start: u32,
	},

	FtdiReadNand {
		#[arg(long)]
		out: PathBuf,

		#[arg(long, default_value_t = 0)]
		start: u32,

		#[arg(long)]
		count: Option<u32>,

		#[arg(long, default_value = "auto")]
		ftdi_desc: String,

		#[arg(long)]
		ftdi_index: Option<i32>,

		#[arg(long, default_value_t = 30_000_000)]
		freq_hz: u32,
	},

	FtdiWriteNand {
		#[arg(long)]
		input: PathBuf,

		#[arg(long, default_value_t = 0)]
		start: u32,

		#[arg(long)]
		count: Option<u32>,

		#[arg(long, default_value = "auto")]
		ftdi_desc: String,

		#[arg(long)]
		ftdi_index: Option<i32>,

		#[arg(long, default_value_t = 30_000_000)]
		freq_hz: u32,
	},

	FtdiList,
}
