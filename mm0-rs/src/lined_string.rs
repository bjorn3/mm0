use std::sync::Arc;
use std::path::PathBuf;
use std::mem;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut, Range, Index};
pub use lsp_types::{Position, Url};
use lsp_types::{TextDocumentContentChangeEvent};

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Span {
  pub start: usize,
  pub end: usize,
}

impl From<Range<usize>> for Span {
  #[inline] fn from(r: Range<usize>) -> Self { Span {start: r.start, end: r.end} }
}

impl From<usize> for Span {
  #[inline] fn from(n: usize) -> Self { Span {start: n, end: n} }
}

impl From<Span> for Range<usize> {
  #[inline] fn from(s: Span) -> Self { s.start..s.end }
}

impl Deref for Span {
  type Target = Range<usize>;
  fn deref(&self) -> &Range<usize> {
    unsafe { mem::transmute(self) }
  }
}

impl DerefMut for Span {
  fn deref_mut(&mut self) -> &mut Range<usize> {
    unsafe { mem::transmute(self) }
  }
}

impl Iterator for Span {
  type Item = usize;
  fn next(&mut self) -> Option<usize> { self.deref_mut().next() }
}
impl DoubleEndedIterator for Span {
  fn next_back(&mut self) -> Option<usize> { self.deref_mut().next_back() }
}

#[derive(Clone)]
pub struct FileRef(Arc<(PathBuf, Url)>);
impl FileRef {
  pub fn new(buf: PathBuf) -> FileRef {
    let u = Url::from_file_path(&buf).expect("bad file path");
    FileRef(Arc::new((buf, u)))
  }
  pub fn from_url(url: Url) -> FileRef {
    FileRef(Arc::new((url.to_file_path().expect("bad URL"), url)))
  }
  pub fn path(&self) -> &PathBuf { &self.0 .0 }
  pub fn url(&self) -> &Url { &self.0 .1 }
}
impl PartialEq for FileRef {
  fn eq(&self, other: &Self) -> bool { self.0 == other.0 }
}
impl Eq for FileRef {}

impl Hash for FileRef {
  fn hash<H: Hasher>(&self, state: &mut H) { self.0.hash(state) }
}

#[derive(Clone, PartialEq, Eq)]
pub struct FileSpan {
  pub file: FileRef,
  pub span: Span,
}

#[derive(Default, Clone)]
pub struct LinedString { s: String, pub lines: Vec<usize> }

impl Index<Span> for LinedString {
  type Output = str;
  fn index(&self, s: Span) -> &str {
    unsafe { std::str::from_utf8_unchecked(&self.as_bytes()[s.start..s.end]) }
  }
}

impl LinedString {

  fn get_lines(s: &str) -> Vec<usize> {
    let mut lines = vec![];
    for (b, c) in s.char_indices() {
      if c == '\n' { lines.push(b + 1) }
    }
    lines
  }

  pub fn to_pos(&self, idx: usize) -> Position {
    let (pos, line) = match self.lines.binary_search(&idx) {
      Ok(n) => (idx, n+1),
      Err(n) => (n.checked_sub(1).map_or(0, |i| self.lines[i]), n)
    };
    Position::new(line as u64, (idx - pos) as u64)
  }

  pub fn to_range(&self, s: Span) -> lsp_types::Range {
    lsp_types::Range {start: self.to_pos(s.start), end: self.to_pos(s.end)}
  }

  pub fn to_loc(&self, fs: &FileSpan) -> lsp_types::Location {
    lsp_types::Location {uri: fs.file.url().clone(), range: self.to_range(fs.span)}
  }

  pub fn num_lines(&self) -> u64 { self.lines.len() as u64 }
  pub fn end(&self) -> Position { self.to_pos(self.s.len()) }

  pub fn to_idx(&self, pos: Position) -> Option<usize> {
    match pos.line.checked_sub(1) {
      None => Some(pos.character as usize),
      Some(n) => self.lines.get(n as usize)
        .map(|&idx| idx + (pos.character as usize))
    }
  }

  pub fn extend(&mut self, s: &str) {
    let len = self.s.len();
    self.s.push_str(s);
    for (b, c) in s.char_indices() {
      if c == '\n' { self.lines.push(b + len + 1) }
    }
  }

  pub fn extend_until<'a>(&mut self, s: &'a str, pos: Position) -> &'a str {
    let end = self.end();
    debug_assert!(end <= pos);
    let (off, tail) = if pos.line == end.line {
      ((pos.character - end.character) as usize, s)
    } else {
      let len = self.s.len();
      self.s.push_str(s);
      let mut it = s.char_indices();
      (pos.character as usize, loop {
        if let Some((b, c)) = it.next() {
          if c == '\n' {
            self.lines.push(b + len + 1);
            if pos.line == self.num_lines() {
              break unsafe { s.get_unchecked(b+1..) }
            }
          }
        } else {break ""}
      })
    };
    let (left, right) = if off < tail.len() {tail.split_at(off)} else {(tail, "")};
    self.extend(left);
    right
  }

  pub fn truncate(&mut self, pos: Position) {
    if let Some(idx) = self.to_idx(pos) {
      if idx < self.s.len() {
        self.s.truncate(idx);
        self.lines.truncate(pos.line as usize);
      }
    }
  }

  pub fn apply_changes(&self, changes: impl Iterator<Item=TextDocumentContentChangeEvent>) ->
      (Position, LinedString) {
    let mut old: LinedString;
    let mut out = LinedString::default();
    let mut uncopied: &str = &self.s;
    let mut first_change = None;
    for TextDocumentContentChangeEvent {range, text: change, ..} in changes {
      if let Some(lsp_types::Range {start, end}) = range {
        if first_change.map_or(true, |c| start < c) { first_change = Some(start) }
        if out.end() > start {
          out.extend(uncopied);
          old = mem::replace(&mut out, LinedString::default());
          uncopied = &old;
        }
        uncopied = out.extend_until(uncopied, end);
        out.truncate(start);
        out.extend(&change);
      } else {
        out = change.into();
        first_change = Some(Position::default());
        uncopied = "";
      }
    }
    out.extend(uncopied);
    if let Some(pos) = first_change {
      let start = out.to_idx(pos).unwrap();
      let from = unsafe { self.s.get_unchecked(start..) };
      let to = unsafe { out.s.get_unchecked(start..) };
      for ((b, c1), c2) in from.char_indices().zip(to.chars()) {
        if c1 != c2 {return (out.to_pos(b + start), out)}
      }
    }
    crate::server::log(format!("{}", out.s));
    (out.end(), out)
  }
}

impl Deref for LinedString {
  type Target = String;
  fn deref(&self) -> &String { &self.s }
}

impl From<String> for LinedString {
  fn from(s: String) -> LinedString {
    LinedString {lines: LinedString::get_lines(&s), s}
  }
}
