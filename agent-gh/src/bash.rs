pub(crate) const MAX_NESTING: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParseError {
    Unparsable,
    TooDeep,
}

pub(crate) fn find_direct_gh(source: &str) -> Result<Option<String>, ParseError> {
    let mut parser = Parser::new(source);
    parser.commands(Until::End)?;
    Ok(parser.found)
}

const RESERVED_PREFIXES: &[&str] = &[
    "!", "if", "then", "else", "elif", "while", "until", "do", "time",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Until {
    End,
    CloseParen,
}

struct Heredoc {
    delimiter: String,
    strip_tabs: bool,
}

#[derive(Default)]
struct Word {
    text: String,
    quoted: bool,
    expanded: bool,
    assignment: Option<bool>,
}

impl Word {
    fn push_equals(&mut self) {
        if self.assignment.is_none() {
            self.assignment =
                Some(!self.quoted && !self.expanded && is_assignment_name(&self.text));
        }
        self.text.push('=');
    }

    fn is_plain(&self, text: &str) -> bool {
        !self.quoted && !self.expanded && self.text == text
    }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    nesting: usize,
    found: Option<String>,
    heredocs: Vec<Heredoc>,
}

impl Parser {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            nesting: 0,
            found: None,
            heredocs: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Result<char, ParseError> {
        let c = self.peek().ok_or(ParseError::Unparsable)?;
        self.pos += 1;
        Ok(c)
    }

    fn nested(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<(), ParseError>,
    ) -> Result<(), ParseError> {
        if self.nesting > MAX_NESTING {
            return Err(ParseError::TooDeep);
        }
        self.nesting += 1;
        let result = parse(self);
        self.nesting -= 1;
        result
    }

    fn commands(&mut self, until: Until) -> Result<(), ParseError> {
        self.nested(|parser| parser.command_list(until))
    }

    fn command_list(&mut self, until: Until) -> Result<(), ParseError> {
        let mut at_command = true;
        let mut depth = 0_usize;
        while let Some(c) = self.skip_blanks() {
            at_command = match c {
                '\n' => {
                    self.pos += 1;
                    self.skip_heredoc_bodies();
                    true
                }
                '#' => {
                    self.skip_comment();
                    at_command
                }
                '&' if self.peek_at(1) == Some('>') => {
                    self.redirection()?;
                    at_command
                }
                ';' | '|' | '&' => {
                    self.skip_operator();
                    true
                }
                '(' if at_command && self.peek_at(1) == Some('(') => {
                    self.pos += 2;
                    self.nested(Self::skip_arithmetic)?;
                    false
                }
                '(' => {
                    self.pos += 1;
                    depth += 1;
                    true
                }
                ')' if depth == 0 && until == Until::CloseParen => {
                    self.pos += 1;
                    return Ok(());
                }
                ')' => {
                    self.pos += 1;
                    depth = depth.saturating_sub(1);
                    true
                }
                '<' | '>' => {
                    self.redirection()?;
                    at_command
                }
                _ => self.word_or_redirection(at_command)?,
            };
        }
        match until {
            Until::End => Ok(()),
            Until::CloseParen => Err(ParseError::Unparsable),
        }
    }

    fn skip_blanks(&mut self) -> Option<char> {
        loop {
            match (self.peek()?, self.peek_at(1)) {
                (' ' | '\t' | '\r', _) => self.pos += 1,
                ('\\', Some('\n')) => self.pos += 2,
                (c, _) => return Some(c),
            }
        }
    }

    fn skip_comment(&mut self) {
        while self.peek().is_some_and(|c| c != '\n') {
            self.pos += 1;
        }
    }

    fn skip_operator(&mut self) {
        while matches!(self.peek(), Some(';' | '|' | '&')) {
            self.pos += 1;
        }
    }

    fn word_or_redirection(&mut self, at_command: bool) -> Result<bool, ParseError> {
        if self.at_fd_redirection() {
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
            self.redirection()?;
            return Ok(at_command);
        }
        let word = self.word()?;
        if word.is_plain("{") {
            return Ok(true);
        }
        if !at_command {
            return Ok(false);
        }
        if word.assignment == Some(true) {
            if word.text.ends_with('=') && self.peek() == Some('(') {
                self.skip_array()?;
            }
            return Ok(true);
        }
        if RESERVED_PREFIXES
            .iter()
            .any(|reserved| word.is_plain(reserved))
        {
            return Ok(true);
        }
        if !word.expanded && is_gh(&word.text) && self.found.is_none() {
            self.found = Some(word.text);
        }
        Ok(false)
    }

    fn at_fd_redirection(&self) -> bool {
        let digits = self
            .chars
            .get(self.pos..)
            .unwrap_or_default()
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        digits > 0 && matches!(self.peek_at(digits), Some('<' | '>'))
    }

    fn redirection(&mut self) -> Result<(), ParseError> {
        if matches!(self.peek(), Some('<' | '>')) && self.peek_at(1) == Some('(') {
            self.pos += 2;
            return self.commands(Until::CloseParen);
        }
        let mut operator = String::new();
        while let Some(c) = self.peek().filter(|c| matches!(c, '<' | '>' | '&' | '-')) {
            operator.push(c);
            self.pos += 1;
        }
        if operator == ">" && self.peek() == Some('|') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.pos += 1;
        }
        if self.peek().is_none_or(is_metachar) {
            return Ok(());
        }
        let target = self.word()?;
        if operator.starts_with("<<") && operator != "<<<" {
            self.heredocs.push(Heredoc {
                delimiter: target.text,
                strip_tabs: operator == "<<-",
            });
        }
        Ok(())
    }

    fn skip_heredoc_bodies(&mut self) {
        for heredoc in std::mem::take(&mut self.heredocs) {
            self.skip_heredoc(&heredoc);
        }
    }

    fn skip_heredoc(&mut self, heredoc: &Heredoc) {
        while let Some(line) = self.read_line() {
            let line = line.strip_suffix('\r').unwrap_or(&line);
            let line = if heredoc.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if line == heredoc.delimiter {
                return;
            }
        }
    }

    fn read_line(&mut self) -> Option<String> {
        self.peek()?;
        let mut line = String::new();
        while let Ok(c) = self.bump() {
            if c == '\n' {
                break;
            }
            line.push(c);
        }
        Some(line)
    }

    fn skip_array(&mut self) -> Result<(), ParseError> {
        self.pos += 1;
        loop {
            match self.skip_blanks().ok_or(ParseError::Unparsable)? {
                ')' => {
                    self.pos += 1;
                    return Ok(());
                }
                '\n' => self.pos += 1,
                '#' => self.skip_comment(),
                c if is_metachar(c) => return Err(ParseError::Unparsable),
                _ => {
                    self.word()?;
                }
            }
        }
    }

    fn word(&mut self) -> Result<Word, ParseError> {
        let mut word = Word::default();
        while let Some(c) = self.peek().filter(|c| !is_metachar(*c)) {
            self.pos += 1;
            match c {
                '\\' => self.escape(&mut word),
                '\'' => self.single_quoted(&mut word)?,
                '"' => self.double_quoted(&mut word)?,
                '$' => self.dollar(&mut word)?,
                '`' => self.backquoted(&mut word)?,
                '=' => word.push_equals(),
                _ => word.text.push(c),
            }
        }
        Ok(word)
    }

    fn escape(&mut self, word: &mut Word) {
        word.quoted = true;
        match self.bump() {
            Ok('\n') | Err(_) => {}
            Ok(c) => word.text.push(c),
        }
    }

    fn single_quoted(&mut self, word: &mut Word) -> Result<(), ParseError> {
        word.quoted = true;
        loop {
            match self.bump()? {
                '\'' => return Ok(()),
                c => word.text.push(c),
            }
        }
    }

    fn double_quoted(&mut self, word: &mut Word) -> Result<(), ParseError> {
        word.quoted = true;
        loop {
            match self.bump()? {
                '"' => return Ok(()),
                '\\' => self.double_quoted_escape(word),
                '$' if matches!(self.peek(), Some('"' | '\'')) => word.text.push('$'),
                '$' => self.dollar(word)?,
                '`' => self.backquoted(word)?,
                c => word.text.push(c),
            }
        }
    }

    fn double_quoted_escape(&mut self, word: &mut Word) {
        match self.peek() {
            Some('\n') => self.pos += 1,
            Some(c @ ('$' | '`' | '"' | '\\')) => {
                self.pos += 1;
                word.text.push(c);
            }
            _ => word.text.push('\\'),
        }
    }

    fn ansi_c_quoted(&mut self, word: &mut Word) -> Result<(), ParseError> {
        word.quoted = true;
        loop {
            match self.bump()? {
                '\'' => return Ok(()),
                '\\' => word.text.push(match self.bump()? {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                }),
                c => word.text.push(c),
            }
        }
    }

    fn dollar(&mut self, word: &mut Word) -> Result<(), ParseError> {
        match (self.peek(), self.peek_at(1)) {
            (Some('\''), _) => {
                self.pos += 1;
                self.ansi_c_quoted(word)
            }
            (Some('"'), _) => {
                self.pos += 1;
                self.double_quoted(word)
            }
            (Some('('), Some('(')) => {
                self.pos += 2;
                word.expanded = true;
                self.nested(Self::skip_arithmetic)
            }
            (Some('('), _) => {
                self.pos += 1;
                word.expanded = true;
                self.commands(Until::CloseParen)
            }
            (Some('{'), _) => {
                self.pos += 1;
                word.expanded = true;
                self.nested(Self::skip_parameter)
            }
            (Some(c), _) if c.is_ascii_alphanumeric() || "_@*#?$!-".contains(c) => {
                word.expanded = true;
                word.text.push('$');
                self.push_variable_name(word, c);
                Ok(())
            }
            _ => {
                word.text.push('$');
                Ok(())
            }
        }
    }

    fn push_variable_name(&mut self, word: &mut Word, first: char) {
        self.pos += 1;
        word.text.push(first);
        if first.is_ascii_alphabetic() || first == '_' {
            while let Some(c) = self
                .peek()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            {
                self.pos += 1;
                word.text.push(c);
            }
        }
    }

    fn skip_arithmetic(&mut self) -> Result<(), ParseError> {
        let mut depth = 0_usize;
        loop {
            match (self.bump()?, depth) {
                ('$', _) => self.dollar(&mut Word::default())?,
                ('`', _) => self.backquoted(&mut Word::default())?,
                ('(', _) => depth += 1,
                (')', 0) if self.peek() == Some(')') => {
                    self.pos += 1;
                    return Ok(());
                }
                (')', _) => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }

    fn skip_parameter(&mut self) -> Result<(), ParseError> {
        let mut scratch = Word::default();
        let mut depth = 1_usize;
        while depth > 0 {
            match self.bump()? {
                '{' => depth += 1,
                '}' => depth -= 1,
                '\\' => self.escape(&mut scratch),
                '\'' => self.single_quoted(&mut scratch)?,
                '"' => self.double_quoted(&mut scratch)?,
                '$' => self.dollar(&mut scratch)?,
                '`' => self.backquoted(&mut scratch)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn backquoted(&mut self, word: &mut Word) -> Result<(), ParseError> {
        word.expanded = true;
        let mut inner = String::new();
        loop {
            match self.bump()? {
                '`' => break,
                '\\' => match self.bump()? {
                    c @ ('`' | '\\' | '$') => inner.push(c),
                    c => {
                        inner.push('\\');
                        inner.push(c);
                    }
                },
                c => inner.push(c),
            }
        }
        let mut parser = Self::new(&inner);
        parser.nesting = self.nesting;
        parser.commands(Until::End)?;
        if self.found.is_none() {
            self.found = parser.found;
        }
        Ok(())
    }
}

fn is_metachar(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\r' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>'
    )
}

fn is_assignment_name(text: &str) -> bool {
    let name = text.strip_suffix('+').unwrap_or(text);
    let name = name.split_once('[').map_or(name, |(name, _)| name);
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_gh(word: &str) -> bool {
    word.rsplit(['/', '\\', ':'])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("gh") || name.eq_ignore_ascii_case("gh.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str) -> Option<String> {
        match find_direct_gh(source) {
            Ok(found) => found,
            Err(error) => panic!("{source:?} should parse: {error:?}"),
        }
    }

    #[test]
    fn finds_direct_invocations() {
        let cases = [
            ("gh issue list", "gh"),
            ("GH_PAGER=cat gh api user", "gh"),
            ("cd repo && gh pr view 1", "gh"),
            ("echo body | gh issue create --body-file -", "gh"),
            ("agent-gh issue list; gh pr list", "gh"),
            ("true || gh auth status", "gh"),
            ("sleep 1 & gh run list", "gh"),
            ("echo \"$(gh auth token)\"", "gh"),
            ("token=`gh auth token`", "gh"),
            ("diff <(gh api repos/a/b) expected.json", "gh"),
            ("/usr/bin/gh issue list", "/usr/bin/gh"),
            (
                "'/c/Program Files/GitHub CLI/gh.exe' pr list",
                "/c/Program Files/GitHub CLI/gh.exe",
            ),
            ("GH.EXE pr list", "GH.EXE"),
            ("\"gh\" issue list", "gh"),
            ("(gh issue list)", "gh"),
            ("{ gh issue list; }", "gh"),
            ("function f { gh pr list; }", "gh"),
            ("if true; then gh issue list; fi", "gh"),
            ("for n in 1 2; do gh issue view \"$n\"; done", "gh"),
            ("while ! gh pr checks; do sleep 5; done", "gh"),
            ("case $x in a) gh issue list;; esac", "gh"),
            ("time gh pr list", "gh"),
            ("2>/dev/null gh issue list", "gh"),
            ("echo x >out.txt 2>&1 && gh pr list", "gh"),
            ("echo x &>/dev/null && gh pr list", "gh"),
            ("echo x 2>&-|gh pr list", "gh"),
            ("cat <<EOF\nbody\nEOF\ngh issue list", "gh"),
            ("cat <<EOF\r\nbody\r\nEOF\r\ngh pr list", "gh"),
            ("cat <<$X\nbody\n$X\ngh issue list", "gh"),
            ("((x = 1 << 2))\ngh issue list", "gh"),
            ("echo one \\\n  && gh issue list", "gh"),
            ("grep \"foo$\" a.txt && gh issue create --title x", "gh"),
            ("gh pr list --state open | grep \"draft$\"", "gh"),
            ("echo \"a $'b\" && gh pr list", "gh"),
            ("echo ${x:-$(gh auth token)}", "gh"),
            ("echo ${x:-\"}\"}; gh pr list", "gh"),
            ("echo $(( $(gh api x) + 1 ))", "gh"),
            ("arr=(\n  one  # first; primary\n)\ngh pr list", "gh"),
        ];
        for (source, word) in cases {
            assert_eq!(found(source).as_deref(), Some(word), "{source:?}");
        }
    }

    #[test]
    fn ignores_commands_without_direct_invocations() {
        let cases = [
            "agent-gh issue list",
            "agent-gh.exe pr create --head feature --body-file pr.md",
            "echo gh issue list",
            "echo 'gh issue list'",
            "git commit -m \"use gh issue list\"",
            "ls # gh issue list",
            "command -v gh",
            "which gh",
            "ghx issue list",
            "$GH issue list",
            "names=(gh git)",
            "(( n=(a>b) ))",
            "cat <<'EOF'\ngh issue list\nEOF",
            "cat <<-EOF\n\tgh issue list\n\tEOF",
            "agent-gh pr create --body \"$(cat <<'EOF'\nRun gh issue list; it's fine\nEOF\n)\"",
            "echo \"gh $(printf '%s' gh)\"",
        ];
        for source in cases {
            assert_eq!(found(source), None, "{source:?}");
        }
    }

    #[test]
    fn rejects_unparsable_commands() {
        for source in [
            "gh issue list \"unclosed",
            "echo 'unclosed",
            "echo $(gh issue list",
            "echo `gh issue list",
            "echo ${HOME",
            "a=(x;y)",
            "a=(<(ls))",
        ] {
            let result = find_direct_gh(source);
            assert_eq!(result, Err(ParseError::Unparsable), "{source:?}");
        }
    }

    #[test]
    fn rejects_nesting_beyond_the_limit() {
        let depth = 10_000;
        for (open, close) in [("$(", ")"), ("$((", "))"), ("${a:-", "}"), ("\"$(", ")\"")] {
            let source = format!("echo {}gh{}", open.repeat(depth), close.repeat(depth));
            let result = find_direct_gh(&source);
            assert_eq!(result, Err(ParseError::TooDeep), "{open}");
        }
    }

    #[test]
    fn accepts_nesting_at_the_limit() {
        let depth = MAX_NESTING;
        let source = format!("echo {}gh{}", "$(".repeat(depth), ")".repeat(depth));
        assert_eq!(found(&source).as_deref(), Some("gh"));
    }
}
