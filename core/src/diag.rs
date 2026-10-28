//! Diagnostics: codes, levels, labels and compiler-style rendering (docs/06).

use crate::json::Json;
use crate::syntax::span::{FileId, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub span: Span,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub level: Level,
    pub message: String,
    /// The first label is the primary one.
    pub labels: Vec<Label>,
    pub help: Option<String>,
    /// A file path to mention when there is no span (e.g. file not found).
    pub path: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            level: Level::Error,
            message: message.into(),
            labels: Vec::new(),
            help: None,
            path: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            level: Level::Warning,
            message: message.into(),
            labels: Vec::new(),
            help: None,
            path: None,
        }
    }

    pub fn primary(mut self, span: Span, text: impl Into<String>) -> Diagnostic {
        self.labels.insert(
            0,
            Label {
                span,
                text: text.into(),
            },
        );
        self
    }

    pub fn secondary(mut self, span: Span, text: impl Into<String>) -> Diagnostic {
        self.labels.push(Label {
            span,
            text: text.into(),
        });
        self
    }

    pub fn help(mut self, text: impl Into<String>) -> Diagnostic {
        self.help = Some(text.into());
        self
    }

    pub fn at_path(mut self, path: impl Into<String>) -> Diagnostic {
        self.path = Some(path.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.level == Level::Error
    }

    pub fn to_json(&self, sm: &SourceMap) -> Json {
        let mut j = Json::object()
            .with("code", self.code)
            .with("level", self.level.as_str())
            .with("message", self.message.as_str());
        if let Some(primary) = self.labels.first() {
            let (file, l1, c1) = sm.position(primary.span.file, primary.span.start);
            let (_, l2, c2) = sm.position(primary.span.file, primary.span.end);
            j.set("file", file.path.as_str());
            j.set("line", l1);
            j.set("column", c1);
            j.set("end_line", l2);
            j.set("end_column", c2);
        } else if let Some(p) = &self.path {
            j.set("file", p.as_str());
        }
        let labels: Vec<Json> = self
            .labels
            .iter()
            .map(|l| {
                let (file, line, col) = sm.position(l.span.file, l.span.start);
                Json::object()
                    .with("file", file.path.as_str())
                    .with("line", line)
                    .with("column", col)
                    .with("text", l.text.as_str())
            })
            .collect();
        j.set("labels", labels);
        j.set("help", self.help.clone());
        j
    }
}

/// A collection of diagnostics for one run.
#[derive(Default, Debug, Clone)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Diagnostics {
        Diagnostics::default()
    }
    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }
    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }
    pub fn error_count(&self) -> usize {
        self.items.iter().filter(|d| d.is_error()).count()
    }
    pub fn warning_count(&self) -> usize {
        self.items.len() - self.error_count()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    /// Sort by file then position so output is stable; errors keep relative order otherwise.
    pub fn sort(&mut self) {
        self.items.sort_by_key(|d| match d.labels.first() {
            Some(l) => (l.span.file.0, l.span.start),
            None => (u32::MAX, 0),
        });
    }
    pub fn render(&self, sm: &SourceMap) -> String {
        let mut out = String::new();
        for d in &self.items {
            out.push_str(&render(d, sm));
            out.push('\n');
        }
        out
    }
}

/// One loaded source file.
pub struct SourceFile {
    pub path: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    fn new(path: String, text: String) -> SourceFile {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile {
            path,
            text,
            line_starts,
        }
    }

    /// 1-based line and column (in characters) of a byte offset.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = self.text[self.line_starts[line]..offset].chars().count() + 1;
        (line + 1, col)
    }

    /// The text of a 1-based line without its newline.
    pub fn line_text(&self, line: usize) -> &str {
        let start = self.line_starts[line - 1];
        let end = self
            .line_starts
            .get(line)
            .map(|e| e - 1)
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches('\r')
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
}

/// All files loaded during one run, addressed by [`FileId`].
#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap::default()
    }

    pub fn add(&mut self, path: impl Into<String>, text: impl Into<String>) -> FileId {
        self.files.push(SourceFile::new(path.into(), text.into()));
        FileId(self.files.len() as u32 - 1)
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }

    pub fn position(&self, id: FileId, offset: u32) -> (&SourceFile, usize, usize) {
        let f = self.get(id);
        let (l, c) = f.line_col(offset as usize);
        (f, l, c)
    }
}

/// Render one diagnostic in the compiler style from docs/06.
pub fn render(d: &Diagnostic, sm: &SourceMap) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}[{}]: {}\n",
        d.level.as_str(),
        d.code,
        d.message
    ));
    if d.labels.is_empty() {
        if let Some(p) = &d.path {
            out.push_str(&format!("  --> {}\n", p));
        }
    } else {
        // Group labels by (file, line) keeping primary first.
        let primary = &d.labels[0];
        let (pf, pl, pc) = sm.position(primary.span.file, primary.span.start);
        out.push_str(&format!("  --> {}:{}:{}\n", pf.path, pl, pc));
        let gutter = d
            .labels
            .iter()
            .map(|l| sm.position(l.span.file, l.span.start).1)
            .max()
            .unwrap_or(1)
            .to_string()
            .len();
        let pad = " ".repeat(gutter);
        out.push_str(&format!("{} |\n", pad));
        let mut rendered: Vec<(FileId, usize)> = Vec::new();
        for (i, label) in d.labels.iter().enumerate() {
            let (file, line, col) = sm.position(label.span.file, label.span.start);
            if rendered.contains(&(label.span.file, line)) {
                continue;
            }
            rendered.push((label.span.file, line));
            if i > 0 && (label.span.file != primary.span.file) {
                out.push_str(&format!(
                    "{} ::: {}:{}:{}\n{} |\n",
                    pad, file.path, line, col, pad
                ));
            }
            let text = file.line_text(line);
            out.push_str(&format!("{:>width$} | {}\n", line, text, width = gutter));
            // All labels on this line, primary with ^, others with -.
            let mut marks: Vec<(usize, usize, char, &str)> = d
                .labels
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    l.span.file == label.span.file
                        && sm.position(l.span.file, l.span.start).1 == line
                })
                .map(|(j, l)| {
                    let (_, _, c) = sm.position(l.span.file, l.span.start);
                    let (_, end_line, end_col) = sm.position(l.span.file, l.span.end);
                    let len = if end_line == line {
                        (end_col - c).max(1)
                    } else {
                        text.chars().count().saturating_sub(c - 1).max(1)
                    };
                    (c, len, if j == 0 { '^' } else { '-' }, l.text.as_str())
                })
                .collect();
            marks.sort_by_key(|m| m.0);
            let mut underline = String::new();
            let mut cursor = 1;
            for (c, len, ch, _) in &marks {
                while cursor < *c {
                    underline.push(' ');
                    cursor += 1;
                }
                for _ in 0..*len {
                    underline.push(*ch);
                    cursor += 1;
                }
            }
            if let Some(last) = marks.last() {
                if !last.3.is_empty() {
                    underline.push(' ');
                    underline.push_str(last.3);
                }
            }
            out.push_str(&format!("{} | {}\n", pad, underline));
            // Remaining labels' texts on their own lines, aligned to their column.
            for (c, _, _, t) in marks.iter().take(marks.len().saturating_sub(1)) {
                if !t.is_empty() {
                    out.push_str(&format!("{} | {}{}\n", pad, " ".repeat(c - 1), t));
                }
            }
        }
        if d.help.is_some() {
            out.push_str(&format!("{} |\n", pad));
        }
        if let Some(h) = &d.help {
            for (i, line) in h.lines().enumerate() {
                if i == 0 {
                    out.push_str(&format!("{} = help: {}\n", pad, line));
                } else {
                    out.push_str(&format!("{}         {}\n", pad, line));
                }
            }
        }
    }
    if d.labels.is_empty() {
        if let Some(h) = &d.help {
            out.push_str(&format!("  = help: {}\n", h));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions() {
        let f = SourceFile::new("x".into(), "ab\ncdé f\n".into());
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(3), (2, 1));
        assert_eq!(f.line_col(7), (2, 4)); // after é (2 bytes)
        assert_eq!(f.line_text(2), "cdé f");
        assert_eq!(f.line_count(), 3);
    }

    #[test]
    fn render_two_labels() {
        let mut sm = SourceMap::new();
        let f = sm.add(
            "pipelines/main.psl",
            "node a : input \"p\"\nmap beats.out -> csv.in\n",
        );
        let d = Diagnostic::error("E0303", "port types differ")
            .primary(Span::new(f, 36, 42), "is `signal`")
            .secondary(Span::new(f, 23, 32), "is `event`")
            .help("convert first");
        let s = render(&d, &sm);
        assert!(s.contains("error[E0303]: port types differ"));
        assert!(s.contains("--> pipelines/main.psl:2:18"));
        assert!(s.contains("2 | map beats.out -> csv.in"));
        assert!(s.contains("  |     ---------    ^^^^^^ is `signal`"));
        assert!(s.contains("  |     is `event`"));
        assert!(s.contains("= help: convert first"));
    }
}
