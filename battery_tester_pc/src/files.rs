use std::io::Write;
use tokio::{
	fs::File,
	io::AsyncWriteExt,
	sync::mpsc::{Receiver, Sender},
};

use crate::{Event, FileCmd, SaveData};

const HEADER_NL: &[u8] = b"dt\tduration\tmillivolts\tmilliamps\n";

pub async fn file_task(event_tx: Sender<Event>, mut file_cmd_rx: Receiver<FileCmd>) {
	let mut persistance: Option<DataPersistance> = None;
	loop {
		let cmd = match file_cmd_rx.recv().await {
			Some(cmd) => cmd,
			None => break,
		};
		match cmd {
			FileCmd::Push(data) => match &mut persistance {
				Some(dp) => dp.new_data(&data).await,
				None => {
					println!("No output file setup for battery data!");
					event_tx.send(Event::FileError).await.unwrap()
				}
			},
			FileCmd::NewFile(file) => match &mut persistance {
				Some(p) => p.new_file(file).await,
				None => {
					persistance = Some(DataPersistance::new(file).await);
				}
			},
			FileCmd::CloseFile => {
				if let Some(mut dp) = persistance.take() {
					dp.flush_reset().await;
				}
			}
			FileCmd::Shutdown => {
				if let Some(mut dp) = persistance.take() {
					dp.flush_reset().await;
				}
				break;
			}
		}
	}
	println!("exiting file_task");
}

pub struct DataPersistance {
	out_buf: Vec<u8>,
	buffered_records: u8,
	out_file: File,
}

impl DataPersistance {
	pub async fn new(mut out_file: File) -> Self {
		out_file.write_all(HEADER_NL).await.unwrap();
		out_file.flush().await.unwrap();
		Self {
			out_buf: Vec::with_capacity(512),
			buffered_records: 0,
			out_file,
		}
	}

	pub async fn new_file(&mut self, out_file: File) {
		self.write_all().await;
		self.out_file = out_file;
		Write::write(&mut self.out_buf, HEADER_NL).unwrap();
		self.write_all().await;
	}

	pub async fn flush_reset(&mut self) {
		println!("flushing out file buffer");
		self.buffered_records = 0;
		self.write_all().await;
	}

	pub async fn new_data(&mut self, data: &SaveData) {
		let mv = data.millivolts;
		let ma = data.milliamps;
		let dt = data.dt;
		let duration = data.duration;
		write!(&mut self.out_buf, "{dt}\t{duration}\t{mv}\t{ma}\n",).unwrap();
		self.buffered_records += 1;
		if self.buffered_records == 10 {
			self.buffered_records = 0;
			self.write_all().await;
			println!("writing to outfile");
		}
	}

	async fn write_all(&mut self) {
		self.out_file.write_all(&self.out_buf).await.unwrap();
		self.out_file.flush().await.unwrap();
		self.out_buf.clear();
	}
}
