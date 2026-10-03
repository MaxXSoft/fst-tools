mod common;

use common::{TestDir, events, expected_events, fixture};
use fstapi::{Reader, Writer, var_dir, var_type, writer_pack_type};
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// Keeps the child alive while the parent closes its FST handles.
#[test]
#[ignore = "subprocess helper, invoked by the inheritance tests"]
fn hold_inherited_handles() {
  let mut byte = [0];
  assert_eq!(io::stdin().read(&mut byte).unwrap(), 0);
}

/// Starts a child while the caller's FST handles are still open.
fn start_child() -> Child {
  Command::new(std::env::current_exe().unwrap())
    .args([
      "--ignored",
      "--exact",
      "hold_inherited_handles",
      "--nocapture",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::inherit())
    .spawn()
    .unwrap()
}

/// Releases the child and confirms it stayed alive throughout the cleanup.
fn release_child(mut child: Child) {
  let running = child.try_wait();
  drop(child.stdin.take());
  let status = child.wait();
  assert!(running.unwrap().is_none(), "child exited before cleanup");
  assert!(status.unwrap().success(), "subprocess helper failed");
}

/// Takes a sorted snapshot without panicking while the child is waiting.
fn files(dir: &TestDir) -> io::Result<Vec<PathBuf>> {
  let mut paths = std::fs::read_dir(dir.path(""))?
    .map(|entry| entry.map(|entry| entry.path()))
    .collect::<io::Result<Vec<_>>>()?;
  paths.sort();
  Ok(paths)
}

#[test]
fn writer_cleanup_does_not_wait_for_a_child_process() {
  for pack in [writer_pack_type::ZLIB, writer_pack_type::LZ4] {
    for repack in [false, true] {
      let dir = TestDir::new();
      let path = dir.path("writer.fst");
      let mut writer = Writer::create(&path, true)
        .unwrap()
        .pack_type(pack)
        .repack_on_close(repack);
      let bit = writer
        .create_var(var_type::VCD_REG, var_dir::OUTPUT, 1, "bit", None)
        .unwrap();
      writer.emit_time_change(0).unwrap();
      writer.emit_value_change(bit, b"1").unwrap();
      writer.emit_time_change(1).unwrap();

      // Spawn while the hierarchy and output files are still open. Windows
      // cannot delete either file if its handle leaks into the child process.
      let child = start_child();
      drop(writer);
      let after_close = files(&dir);
      // Release the child before asserting, including when read_dir failed.
      release_child(child);
      assert_eq!(
        after_close.unwrap(),
        vec![path.clone()],
        "pack {pack}, repack {repack}"
      );
      let reader = Reader::open(path).unwrap();
      assert_eq!((reader.start_time(), reader.end_time()), (0, 1));
    }
  }
}

#[test]
fn reader_cleanup_does_not_wait_for_a_child_process() {
  for pack in [writer_pack_type::ZLIB, writer_pack_type::LZ4] {
    let dir = TestDir::new();
    let path = dir.path("reader.fst");
    let handles = fixture(&path, pack, true, true);
    let mut reader = Reader::open(&path).unwrap();
    // Walking the hierarchy creates its scratch file in addition to the
    // unpacked file created when the repacked waveform is opened.
    assert_eq!(reader.vars().map(Result::unwrap).count(), 5);
    reader.set_mask_all();
    assert_eq!(events(&mut reader), expected_events(handles, false));

    let child = start_child();
    drop(reader);
    let after_close = files(&dir);
    release_child(child);
    assert_eq!(after_close.unwrap(), vec![path], "pack {pack}");
  }
}
