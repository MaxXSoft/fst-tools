mod common;

use common::*;
use fstapi::{Hier, Reader, var_dir, var_type, writer_pack_type};
use std::ffi::CStr;
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn controlled_traversal_cancels_fixed_variable_and_frame_callbacks_and_restarts() {
  let dir = TestDir::new();
  let path = dir.path("cancel.fst");
  let mut writer = fstapi::Writer::create(&path, true).unwrap();
  let fixed = writer
    .create_var(var_type::VCD_WIRE, var_dir::OUTPUT, 1, "fixed", None)
    .unwrap();
  let variable = writer
    .create_var(var_type::GEN_STRING, var_dir::OUTPUT, 0, "bytes", None)
    .unwrap();
  for time in 0..100 {
    writer.emit_time_change(time).unwrap();
    writer
      .emit_value_change(fixed, if time % 2 == 0 { b"0" } else { b"1" })
      .unwrap();
    writer
      .emit_var_len_value_change(variable, b"payload")
      .unwrap();
    if time == 50 {
      writer.flush();
    }
  }
  writer.emit_time_change(100).unwrap();
  drop(writer);
  for (handle, start) in [(fixed, 0), (variable, 0), (fixed, 80)] {
    let mut reader = Reader::open(&path).unwrap();
    reader.set_mask(handle);
    reader.set_time_range_limit(start, 100);
    let mut full = 0;
    reader.for_each_block(|_, _, _, _| full += 1).unwrap();
    assert!(full > 1);
    for _ in 0..8 {
      let mut calls = 0;
      let complete = reader
        .for_each_block_controlled(|_, _, _, _| {
          calls += 1;
          ControlFlow::Break(())
        })
        .unwrap();
      assert!(!complete);
      assert_eq!(calls, 1);
    }
    let mut calls = 0;
    assert!(
      reader
        .for_each_block_controlled(|_, _, _, _| {
          calls += 1;
          ControlFlow::Continue(())
        })
        .unwrap()
    );
    assert_eq!(calls, full);
  }
}

#[test]
fn hierarchy_items_keep_their_records_and_strings_after_iteration() {
  let dir = TestDir::new();
  let path = dir.path("owned-hierarchy.fst");
  let handles = fixture(&path, writer_pack_type::LZ4, true, false);
  let mut reader = Reader::open(&path).unwrap();
  let items: Vec<_> = reader.hiers().collect();

  let scopes: Vec<_> = items
    .iter()
    .filter_map(|item| match item {
      Hier::Scope(scope) => Some(scope),
      _ => None,
    })
    .collect();
  assert_eq!(scopes.len(), 2);
  assert_eq!(scopes[0].name().unwrap(), "top");
  assert_eq!(scopes[0].component().unwrap(), "top");
  assert_eq!(scopes[1].name().unwrap(), "child");
  assert_eq!(scopes[1].component().unwrap(), "child");
  assert_eq!(
    unsafe { CStr::from_ptr(scopes[0].name_raw()) }.to_bytes(),
    b"top"
  );
  assert_eq!(
    unsafe { CStr::from_ptr(scopes[0].component_raw()) }.to_bytes(),
    b"top"
  );

  let vars: Vec<_> = items
    .iter()
    .filter_map(|item| match item {
      Hier::Var(var) => Some(var),
      _ => None,
    })
    .collect();
  assert_eq!(vars.len(), 5);
  assert_eq!(vars[0].name().unwrap(), "vector");
  assert_eq!(vars[0].ty(), var_type::VCD_REG);
  assert_eq!(vars[0].direction(), var_dir::OUTPUT);
  assert_eq!(vars[0].length(), 8);
  assert_eq!(vars[0].handle(), handles[0]);
  assert!(!vars[0].is_alias());
  assert_eq!(vars[4].name().unwrap(), "alias");
  assert!(vars[4].is_alias());
  assert_eq!(vars[4].handle(), handles[0]);
  assert_eq!(
    unsafe { CStr::from_ptr(vars[0].name_raw()) }.to_bytes(),
    b"vector"
  );

  let attrs: Vec<_> = items
    .iter()
    .filter_map(|item| match item {
      Hier::AttrBegin(attr) => Some(attr),
      _ => None,
    })
    .collect();
  assert_eq!(attrs.len(), 1);
  assert_eq!(attrs[0].name().unwrap(), "round-trip fixture");
  assert_eq!(
    unsafe { CStr::from_ptr(attrs[0].name_raw()) }.to_bytes(),
    b"round-trip fixture"
  );
}

#[test]
fn collected_variables_keep_their_original_metadata() {
  let dir = TestDir::new();
  let path = dir.path("owned-variables.fst");
  let handles = fixture(&path, writer_pack_type::LZ4, true, false);
  let mut reader = Reader::open(&path).unwrap();
  let variables = reader.vars().collect::<fstapi::Result<Vec<_>>>().unwrap();
  assert_eq!(variables.len(), 5);
  let (name, var) = &variables[0];
  assert_eq!(name, "top.vector");
  assert_eq!(var.name().unwrap(), "vector");
  assert_eq!(var.ty(), var_type::VCD_REG);
  assert_eq!(var.handle(), handles[0]);
  assert!(!var.is_alias());
  let (name, var) = &variables[4];
  assert_eq!(name, "top.child.alias");
  assert_eq!(var.name().unwrap(), "alias");
  assert_eq!(var.handle(), handles[0]);
  assert!(var.is_alias());
}

#[test]
fn callback_panics_resume_after_c_cleanup_and_reader_can_be_reused() {
  let dir = TestDir::new();
  let path = dir.path("callback-panic.fst");
  let handles = fixture(&path, writer_pack_type::LZ4, true, false);

  // Exercise both C callback signatures independently.
  for handle in [handles[0], handles[2]] {
    let mut reader = Reader::open(&path).unwrap();
    reader.clear_mask_all();
    reader.set_mask(handle);
    let mut calls = 0;
    let panic = catch_unwind(AssertUnwindSafe(|| {
      reader.for_each_block(|_, _, _, _| {
        calls += 1;
        panic!("callback panic payload");
      })
    }))
    .expect_err("the callback panic must be resumed on the Rust side");
    assert_eq!(
      panic.downcast_ref::<&str>(),
      Some(&"callback panic payload")
    );
    assert_eq!(calls, 1, "skip further callbacks after the first panic");

    let expected: Vec<_> = expected_events(handles, false)
      .into_iter()
      .filter(|event| event.handle == handle)
      .collect();
    assert_eq!(events(&mut reader), expected);
  }
}
