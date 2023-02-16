use std::iter;
use tabled::object::Rows;
use tabled::{Alignment, Modify, Panel, Style, Table, Tabled};

/// Item that represents the content of a [`Section`].
pub enum Item<T> {
  One(T),
  Many(Vec<T>),
}

/// Section of waveform information.
pub trait Section {
  /// Type of the item in the current section.
  type Item;

  /// Returns section name.
  fn name(&self) -> &str;

  /// Returns the item in the current section.
  fn item(&mut self) -> Item<&Self::Item>;
}

/// Trait for printing a sections.
pub trait Print {
  /// Prints to stdout.
  fn print(&mut self);
}

impl<S, I> Print for S
where
  S: Section<Item = I>,
  I: Tabled,
{
  fn print(&mut self) {
    let mut table = match self.item() {
      Item::One(i) => {
        let mut builder = Table::builder(iter::once(i)).index();
        builder.set_index(0).transpose();
        builder.build()
      }
      Item::Many(is) => Table::new(is),
    };
    println!(
      "{}",
      table
        .with(Style::modern())
        .with(Panel::header(self.name()))
        .with(Modify::new(Rows::first()).with(Alignment::center()))
    );
  }
}

impl Print for Vec<Box<dyn Print>> {
  fn print(&mut self) {
    for (i, s) in self.iter_mut().enumerate() {
      if i != 0 {
        println!()
      }
      s.print()
    }
  }
}
