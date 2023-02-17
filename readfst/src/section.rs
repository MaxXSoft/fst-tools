use tabled::builder::Builder;
use tabled::object::FirstRow;
use tabled::{Alignment, Modify, Panel, Style, Table, Tabled};

/// Item that represents the content of a [`Section`].
pub enum Item<'a, T> {
  One(&'a T),
  Many(&'a [T]),
}

/// Section of waveform information.
pub trait Section {
  /// Type of the item in the current section.
  type Item;

  /// Returns section name.
  fn name() -> &'static str;

  /// Returns the item in the current section.
  fn item(&self) -> Item<Self::Item>;
}

/// Trait for converting to [`Table`].
pub trait ToTable {
  /// Converts to table.
  fn to_table(&self) -> Table;
}

impl<'a, T> ToTable for Item<'a, T>
where
  T: Tabled,
{
  fn to_table(&self) -> Table {
    match self {
      Item::One(i) => {
        let mut builder = Table::builder([i]).index();
        builder.set_index(0).transpose();
        builder.build()
      }
      Item::Many(is) if is.is_empty() => {
        let mut builder = Builder::default();
        builder.add_record(["None"]);
        builder.build()
      }
      Item::Many(is) => Table::new(*is),
    }
  }
}

impl<S, I> ToTable for S
where
  S: Section<Item = I>,
  I: Tabled,
{
  fn to_table(&self) -> Table {
    let mut table = self.item().to_table();
    table
      .with(Panel::header(Self::name()))
      .with(Modify::new(FirstRow).with(Alignment::center()));
    table
  }
}

/// Trait for printing to stdout.
pub trait Print {
  /// Prints to stdout.
  fn print(&self);
}

impl<T> Print for T
where
  T: ToTable,
{
  fn print(&self) {
    let mut table = self.to_table();
    println!("{}", table.with(Style::modern()));
  }
}

impl Print for Vec<Box<dyn Print>> {
  fn print(&self) {
    for (i, s) in self.iter().enumerate() {
      if i != 0 {
        println!()
      }
      s.print()
    }
  }
}
