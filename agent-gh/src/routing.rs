use anyhow::Result;
use anyhow::bail;
use std::ffi::OsString;
use std::fmt;

#[derive(Default)]
pub(crate) struct Rules(Vec<Rule>);

struct Rule {
    effect: Effect,
    positionals: Vec<String>,
    flags: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Effect {
    Bot,
    Except,
}

impl Rules {
    pub(crate) fn parse(entries: &[String]) -> Result<Self> {
        entries
            .iter()
            .map(|entry| Rule::parse(entry))
            .collect::<Result<_>>()
            .map(Self)
    }

    pub(crate) fn selects_bot(&self, args: &[OsString]) -> bool {
        let matching = |effect| {
            self.0
                .iter()
                .any(|rule| rule.effect == effect && rule.matches(args))
        };
        matching(Effect::Bot) && !matching(Effect::Except)
    }
}

impl fmt::Display for Rules {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[")?;
        for (index, rule) in self.0.iter().enumerate() {
            let separator = if index == 0 { "" } else { ", " };
            write!(formatter, "{separator}\"{rule}\"")?;
        }
        write!(formatter, "]")
    }
}

impl Rule {
    fn parse(entry: &str) -> Result<Self> {
        let trimmed = entry.trim_start();
        if trimmed.is_empty() {
            bail!("run_as_bot entry {entry:?} is empty");
        }
        let (effect, tokens) = match trimmed.strip_prefix('!') {
            Some(rest) => (Effect::Except, rest),
            None => (Effect::Bot, trimmed),
        };
        let mut positionals = Vec::new();
        let mut flags = Vec::new();
        for token in tokens.split_whitespace() {
            if token.starts_with('-') {
                flags.push(token.to_owned());
            } else if flags.is_empty() {
                positionals.push(token.to_owned());
            } else {
                bail!("run_as_bot entry {entry:?} has the positional word {token:?} after a flag");
            }
        }
        if positionals.is_empty() {
            bail!("run_as_bot entry {entry:?} must start with a positional word");
        }
        Ok(Self {
            effect,
            positionals,
            flags,
        })
    }

    fn matches(&self, args: &[OsString]) -> bool {
        let matches_arg =
            |token: &str, arg: &OsString| arg.to_str().is_some_and(|arg| glob(token, arg));
        self.positionals.len() <= args.len()
            && self
                .positionals
                .iter()
                .zip(args)
                .all(|(token, arg)| matches_arg(token, arg))
            && self
                .flags
                .iter()
                .all(|token| args.iter().any(|arg| matches_arg(token, arg)))
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.effect == Effect::Except {
            write!(formatter, "!")?;
        }
        let tokens: Vec<&str> = self
            .positionals
            .iter()
            .chain(&self.flags)
            .map(String::as_str)
            .collect();
        write!(formatter, "{}", tokens.join(" "))
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    let mut segments = pattern.split('*');
    let Some(mut rest) = segments.next().and_then(|first| text.strip_prefix(first)) else {
        return false;
    };
    let Some(last) = segments.next_back() else {
        return rest.is_empty();
    };
    for segment in segments {
        match rest.split_once(segment) {
            Some((_, after)) => rest = after,
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(entries: &[&str]) -> Rules {
        let entries: Vec<String> = entries.iter().map(|&entry| entry.to_owned()).collect();
        Rules::parse(&entries).expect("entries are valid")
    }

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    fn error(entry: &str) -> String {
        match Rules::parse(&[entry.to_owned()]) {
            Ok(_) => panic!("entry {entry:?} should be rejected"),
            Err(error) => format!("{error:#}"),
        }
    }

    #[test]
    fn glob_matches_whole_arguments() {
        let cases = [
            ("comment", "comment", true),
            ("comment", "comments", false),
            ("comment", "Comment", false),
            ("comment", "", false),
            ("", "", true),
            ("", "x", false),
            ("*", "", true),
            ("*", "any/thing", true),
            ("repos/*", "repos/", true),
            ("repos/*", "repos/o/r/issues", true),
            ("repos/*", "/repos/o", false),
            ("repos/*/comments", "repos//comments", true),
            ("repos/*/comments", "repos/o/r/issues/1/comments", true),
            ("repos/*/comments", "repos/o/r/issues/1/comments/2", false),
            ("repos/*/comments", "Repos/o/r/comments", false),
            (
                "repos/*/comments/*",
                "repos/o/r/pulls/1/comments/2/replies",
                true,
            ),
            ("repos/*/comments/*", "repos/o/r/issues/1/comments", false),
            ("*a*b*", "ab", true),
            ("*a*b*", "xxaxxbxx", true),
            ("*a*b*", "ba", false),
            ("a*a", "a", false),
            ("a*a", "aa", true),
            ("a**b", "ab", true),
            ("-*", "--comment", true),
            ("--comment", "--comment=x", false),
            ("é*", "éa", true),
        ];
        for (pattern, text, expected) in cases {
            assert_eq!(
                glob(pattern, text),
                expected,
                "{pattern:?} against {text:?}"
            );
        }
    }

    #[test]
    fn positional_words_match_leading_arguments_in_order() {
        let rule = rules(&["pr review"]);
        let cases: &[(&[&str], bool)] = &[
            (&["pr", "review"], true),
            (&["pr", "review", "1", "--approve"], true),
            (&["pr"], false),
            (&[], false),
            (&["review", "pr"], false),
            (&["pr", "-R", "o/r", "review"], false),
            (&["issue", "pr", "review"], false),
        ];
        for (words, expected) in cases {
            assert_eq!(rule.selects_bot(&args(words)), *expected, "{words:?}");
        }
    }

    #[test]
    fn flag_tokens_match_any_argument() {
        let rule = rules(&["pr review -c --body*"]);
        let cases: &[(&[&str], bool)] = &[
            (&["pr", "review", "1", "-c", "--body=x"], true),
            (&["pr", "review", "--body", "x", "1", "-c"], true),
            (&["pr", "review", "1", "-c"], false),
            (&["pr", "review", "1", "--body", "x"], false),
            (&["pr", "review", "1", "-bc", "--body", "x"], false),
            (&["pr", "review", "--approve", "--body", "-c"], true),
        ];
        for (words, expected) in cases {
            assert_eq!(rule.selects_bot(&args(words)), *expected, "{words:?}");
        }
    }

    #[test]
    fn non_utf8_argument_matches_no_token() {
        #[cfg(windows)]
        let invalid = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[0xD800])
        };
        #[cfg(unix)]
        let invalid = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![0xFF])
        };
        assert!(invalid.to_str().is_none());

        let cases = [
            (rules(&["*"]), vec![invalid.clone()]),
            (rules(&["pr *"]), vec!["pr".into(), invalid.clone()]),
            (rules(&["pr -*"]), vec!["pr".into(), invalid.clone()]),
        ];
        for (rule, words) in cases {
            assert!(!rule.selects_bot(&words), "{rule} against {words:?}");
        }
        assert!(rules(&["pr"]).selects_bot(&["pr".into(), invalid]));
    }

    #[test]
    fn exceptions_override_matching_entries_in_any_order() {
        let cases: &[(&[&str], &[&str], bool)] = &[
            (&["pr *", "!pr merge"], &["pr", "merge", "1"], false),
            (&["!pr merge", "pr *"], &["pr", "merge", "1"], false),
            (&["pr *", "!pr merge"], &["pr", "view", "1"], true),
            (&["!pr merge", "pr *"], &["pr", "view", "1"], true),
            (
                &["pr comment", "!pr comment --web"],
                &["pr", "comment", "1", "--web"],
                false,
            ),
            (
                &["pr comment", "!pr comment --web"],
                &["pr", "comment", "1"],
                true,
            ),
            (&["!pr merge"], &["pr", "merge"], false),
            (&["!pr merge"], &["pr", "view"], false),
            (&[], &["pr", "view"], false),
        ];
        for (entries, words, expected) in cases {
            assert_eq!(
                rules(entries).selects_bot(&args(words)),
                *expected,
                "{entries:?} against {words:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_entries() {
        let cases = [
            ("", r#"run_as_bot entry "" is empty"#),
            (" \t ", r#"run_as_bot entry " \t " is empty"#),
            (
                "!",
                r#"run_as_bot entry "!" must start with a positional word"#,
            ),
            (
                "--comment",
                r#"run_as_bot entry "--comment" must start with a positional word"#,
            ),
            (
                "!--comment",
                r#"run_as_bot entry "!--comment" must start with a positional word"#,
            ),
            (
                "pr --comment review",
                r#"run_as_bot entry "pr --comment review" has the positional word "review" after a flag"#,
            ),
        ];
        for (entry, expected) in cases {
            assert_eq!(error(entry), expected, "{entry:?}");
        }
    }

    #[test]
    fn displays_entries_with_normalized_whitespace() {
        let rules = rules(&[" pr \t comment ", "! pr merge", "!pr review --approve"]);
        assert_eq!(
            rules.to_string(),
            r#"["pr comment", "!pr merge", "!pr review --approve"]"#
        );
        assert_eq!(Rules::default().to_string(), "[]");
    }

    #[test]
    fn exception_marker_follows_leading_whitespace_and_may_precede_whitespace() {
        for exception in ["! pr merge", " !pr merge", "\n!pr merge"] {
            let rules = rules(&["pr *", exception]);
            assert!(!rules.selects_bot(&args(&["pr", "merge"])), "{exception:?}");
            assert!(rules.selects_bot(&args(&["pr", "view"])), "{exception:?}");
        }
    }

    #[test]
    fn only_the_first_exclamation_mark_marks_an_exception() {
        let rules = rules(&["*", "!!pr"]);
        assert!(!rules.selects_bot(&args(&["!pr"])));
        assert!(rules.selects_bot(&args(&["pr"])));
        assert_eq!(rules.to_string(), r#"["*", "!!pr"]"#);
    }

    #[test]
    fn exclamation_mark_after_the_first_word_is_literal() {
        let rules = rules(&["pr !x"]);
        assert!(rules.selects_bot(&args(&["pr", "!x"])));
        assert!(!rules.selects_bot(&args(&["pr", "x"])));
        assert_eq!(rules.to_string(), r#"["pr !x"]"#);
    }

    #[test]
    fn newline_separates_tokens() {
        let rules = rules(&["pr\ncomment\n--web"]);
        assert!(rules.selects_bot(&args(&["pr", "comment", "1", "--web"])));
        assert!(!rules.selects_bot(&args(&["pr", "comment", "1"])));
        assert_eq!(rules.to_string(), r#"["pr comment --web"]"#);
    }
}
