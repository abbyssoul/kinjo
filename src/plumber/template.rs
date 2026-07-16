//! The compiled form of an action's command template.
//!
//! A template is turned into an argument vector *shape* once, when the command
//! file is loaded, and never re-read from its raw text afterwards. That single
//! decision buys two things:
//!
//!  - **Validation happens at load time.** Malformed quoting, dangling escapes,
//!    and unknown or malformed placeholders are rejected by `list-commands` and
//!    reported as startup warnings, instead of surfacing only when a user picks
//!    the action.
//!  - **Interpolation cannot inject arguments.** Token boundaries are fixed by
//!    [`CommandTemplate::compile`] before any discovered value exists. Rendering
//!    only ever concatenates text *inside* an already-decided token, so a
//!    service name full of spaces, quotes, or braces cannot add, remove, or
//!    split an argument. This is the whole reason templates are not passed to a
//!    shell.

use std::{iter::Peekable, str::Chars};

use color_eyre::eyre::{Result, eyre};

use crate::discovery::Entry;

use super::is_supported_field;

/// One piece of a compiled argument: either literal text taken from the command
/// file, or a reference to a service field resolved against a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fragment {
    Literal(String),
    Field(String),
}

/// One argument of a compiled template. A token with no fragments is a quoted
/// empty argument (`cmd ""`), which renders to an empty string rather than
/// disappearing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Token {
    fragments: Vec<Fragment>,
}

/// A validated, executable command template: the fixed argv shape of an action.
///
/// Construct one with [`CommandTemplate::compile`]; there is no way to build an
/// unvalidated template, so holding a `CommandTemplate` is proof that its
/// quoting, escapes, and placeholders are well formed and that every field it
/// names is one the discovery layer can actually supply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTemplate {
    tokens: Vec<Token>,
}

impl CommandTemplate {
    /// Compile `command` into its argv shape, or explain why it cannot be one.
    ///
    /// The grammar is deliberately *not* a shell. Arguments are separated by
    /// unquoted whitespace; single and double quotes remove their delimiters and
    /// preserve their contents; adjacent quoted and unquoted fragments form one
    /// argument; a backslash escapes exactly the next Unicode scalar, inside or
    /// outside quotes. There is no expansion, no environment substitution, no
    /// pipeline, and no redirection.
    ///
    /// Placeholders are `{field}`. `{{` emits a literal `{`, and a lone `}`
    /// stays literal for compatibility with templates such as
    /// `echo {hostname}}`.
    pub fn compile(command: &str) -> Result<Self> {
        let tokens = tokenize(command)?;
        let Some(program) = tokens.first() else {
            return Err(eyre!("action command is empty"));
        };
        // An empty argv[0] can never name a program, so reject it here rather
        // than letting every candidate fail identically at spawn time. A
        // placeholder program name is allowed: it is only knowable per record,
        // and `CommandAction::prepare` checks the rendered result.
        if program.fragments.is_empty() {
            return Err(eyre!("action command has an empty program name"));
        }
        Ok(Self { tokens })
    }

    /// The service fields this template interpolates, in template order and
    /// with duplicates retained.
    pub(super) fn fields(&self) -> impl Iterator<Item = &str> {
        self.tokens
            .iter()
            .flat_map(|token| token.fragments.iter())
            .filter_map(|fragment| match fragment {
                Fragment::Field(name) => Some(name.as_str()),
                Fragment::Literal(_) => None,
            })
    }

    /// Whether this template interpolates `field`, spelled exactly. Callers ask
    /// about `address` and `port`, which have no aliases.
    pub(super) fn references(&self, field: &str) -> bool {
        self.fields().any(|name| name == field)
    }

    /// Render this template against `record`, producing one argument per token.
    ///
    /// The returned vector always has exactly as many entries as the template
    /// has tokens: field values fill tokens, they never create them.
    pub(super) fn render(&self, record: &Entry) -> Result<Vec<String>> {
        self.tokens
            .iter()
            .map(|token| token.render(record))
            .collect()
    }
}

impl Token {
    fn render(&self, record: &Entry) -> Result<String> {
        let mut argument = String::new();
        for fragment in &self.fragments {
            match fragment {
                Fragment::Literal(text) => argument.push_str(text),
                Fragment::Field(field) => {
                    // Compilation proved the field is supported, so a `None`
                    // here means this record does not carry it. Rules only offer
                    // candidates whose fields all resolve, so reaching this is a
                    // caller error rather than a configuration one.
                    let Some(value) = record.field(field) else {
                        return Err(eyre!(
                            "service field `{field}` is unavailable for `{}`",
                            record.name
                        ));
                    };
                    argument.push_str(&value);
                }
            }
        }
        Ok(argument)
    }
}

/// Split `command` into compiled tokens, resolving quoting, escapes, and
/// placeholders in one pass.
///
/// One pass is not an optimization, it is a correctness requirement: quoting and
/// placeholders are not separable layers. Scanning for placeholders after
/// unquoting would make `\{name}` — an escaped, literal brace — look exactly
/// like a placeholder.
fn tokenize(command: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut literal = String::new();
    // Whether a token is being built. Tracked separately from the buffers so a
    // quoted empty argument (`cmd ""`) survives: it has started but has no text.
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        match (quote, ch) {
            // Closing the active quote. Matched first so that the *other* quote
            // style stays literal text inside it (`"it's"`).
            (Some(active), ch) if ch == active => quote = None,
            (_, '\\') => {
                let Some(escaped) = chars.next() else {
                    return Err(eyre!("dangling `\\` at the end of `{command}`"));
                };
                literal.push(escaped);
                started = true;
            }
            (_, '{') => {
                if chars.next_if_eq(&'{').is_some() {
                    literal.push('{');
                } else {
                    let field = read_placeholder(&mut chars, command)?;
                    flush_literal(&mut literal, &mut fragments);
                    fragments.push(Fragment::Field(field));
                }
                started = true;
            }
            (None, '"' | '\'') => {
                quote = Some(ch);
                started = true;
            }
            (None, ch) if ch.is_whitespace() => {
                if started {
                    flush_literal(&mut literal, &mut fragments);
                    tokens.push(Token {
                        fragments: std::mem::take(&mut fragments),
                    });
                    started = false;
                }
            }
            (_, ch) => {
                literal.push(ch);
                started = true;
            }
        }
    }

    if let Some(active) = quote {
        return Err(eyre!("unterminated `{active}` quote in `{command}`"));
    }
    if started {
        flush_literal(&mut literal, &mut fragments);
        tokens.push(Token { fragments });
    }
    Ok(tokens)
}

/// Read a placeholder body, having just consumed its opening `{`.
fn read_placeholder(chars: &mut Peekable<Chars<'_>>, command: &str) -> Result<String> {
    let mut field = String::new();
    loop {
        let Some(ch) = chars.next() else {
            return Err(eyre!("unterminated placeholder `{{` in `{command}`"));
        };
        match ch {
            '}' => break,
            // `{name{` cannot be a field and is far more likely a typo than an
            // intent, so it is rejected rather than guessed at.
            '{' => return Err(eyre!("nested `{{` inside a placeholder in `{command}`")),
            _ => field.push(ch),
        }
    }
    if field.is_empty() {
        return Err(eyre!("empty placeholder `{{}}` in `{command}`"));
    }
    if !is_supported_field(&field) {
        return Err(eyre!(
            "unknown service field `{field}` in `{command}`; \
             supported fields are name, service_type (or type), domain, \
             hostname, address, port, and txt.<key>"
        ));
    }
    Ok(field)
}

/// Move any buffered literal text into `fragments`. Empty text adds no fragment,
/// which is what keeps an empty token empty.
fn flush_literal(literal: &mut String, fragments: &mut Vec<Fragment>) {
    if !literal.is_empty() {
        fragments.push(Fragment::Literal(std::mem::take(literal)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rendered argv of `template` against a fully populated record.
    fn render(template: &str) -> Vec<String> {
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        record.addresses = vec!["192.0.2.5".parse().unwrap()];
        record.port = Some(22);
        record.txt.insert("path".to_string(), "/admin".to_string());
        CommandTemplate::compile(template)
            .expect("template compiles")
            .render(&record)
            .expect("all fields resolve")
    }

    fn error(template: &str) -> String {
        CommandTemplate::compile(template)
            .expect_err("template must be rejected")
            .to_string()
    }

    #[test]
    fn unquoted_whitespace_separates_arguments() {
        assert_eq!(
            render("ssh  alpha\tbeta\ngamma"),
            ["ssh", "alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn quotes_are_removed_and_contents_preserved() {
        assert_eq!(
            render(r#"printf "two words" 'single quoted'"#),
            ["printf", "two words", "single quoted"]
        );
    }

    #[test]
    fn the_other_quote_style_stays_literal_inside_a_quote() {
        assert_eq!(
            render(r#"echo "it's" 'say "hi"'"#),
            ["echo", "it's", r#"say "hi""#]
        );
    }

    #[test]
    fn adjacent_fragments_form_one_argument() {
        assert_eq!(render(r#"echo a"b"'c'd"#), ["echo", "abcd"]);
        // Including when a placeholder is one of the fragments.
        assert_eq!(
            render(r#"ssh user@"{hostname}":22"#),
            ["ssh", "user@alpha.local:22"]
        );
    }

    #[test]
    fn backslash_escapes_the_next_scalar_inside_and_outside_quotes() {
        assert_eq!(render(r"echo one\ arg"), ["echo", "one arg"]);
        assert_eq!(render(r#"echo "a\"b" 'c\'d'"#), ["echo", r#"a"b"#, "c'd"]);
        // An escaped brace is literal text, not the start of a placeholder.
        assert_eq!(render(r"echo \{hostname\}"), ["echo", "{hostname}"]);
        // Escaping a non-special scalar yields that scalar.
        assert_eq!(render(r"echo \z"), ["echo", "z"]);
    }

    #[test]
    fn quoted_empty_arguments_are_preserved_including_at_the_end() {
        assert_eq!(render(r#"cmd "" next"#), ["cmd", "", "next"]);
        assert_eq!(render("cmd ''"), ["cmd", ""]);
        assert_eq!(render(r#"cmd '' """#), ["cmd", "", ""]);
    }

    #[test]
    fn dangling_backslash_is_rejected() {
        assert!(error(r"echo \").contains("dangling"));
        assert!(error(r"echo 'a\").contains("dangling"));
    }

    #[test]
    fn unterminated_quote_is_rejected() {
        assert!(error("echo 'alpha").contains("unterminated `'` quote"));
        assert!(error(r#"echo "alpha"#).contains("unterminated `\"` quote"));
    }

    #[test]
    fn double_brace_emits_a_literal_brace_and_a_lone_close_brace_stays_literal() {
        assert_eq!(render("echo {{hostname}"), ["echo", "{hostname}"]);
        assert_eq!(render("echo }"), ["echo", "}"]);
        assert_eq!(render("echo {hostname}}"), ["echo", "alpha.local}"]);
    }

    #[test]
    fn malformed_placeholders_are_rejected() {
        assert!(error("echo {name").contains("unterminated placeholder"));
        assert!(error("echo {}").contains("empty placeholder"));
        assert!(error("echo {na{me}}").contains("nested"));
        assert!(error("echo {nonexistent_field}").contains("unknown service field"));
        // A near-miss on a real field name is still a rejection, not a silent
        // never-matching rule.
        assert!(error("echo {service_typ}").contains("unknown service field"));
    }

    #[test]
    fn every_supported_field_and_alias_compiles_and_renders() {
        assert_eq!(
            render(
                "run {name} {type} {service_type} {domain} {hostname} {address} {port} {txt.path}"
            ),
            [
                "run",
                "alpha",
                "_ssh._tcp",
                "_ssh._tcp",
                "local",
                "alpha.local",
                "192.0.2.5",
                "22",
                "/admin",
            ]
        );
    }

    #[test]
    fn arbitrary_txt_keys_are_supported_but_a_bare_txt_is_not() {
        assert!(CommandTemplate::compile("echo {txt.anything-at-all}").is_ok());
        // `txt.txt.path` is a TXT key literally named `txt.path`, not a nested
        // lookup, so it is a supported field name.
        assert!(CommandTemplate::compile("echo {txt.txt.path}").is_ok());
        assert!(error("echo {txt}").contains("unknown service field"));
        assert!(error("echo {txt.}").contains("unknown service field"));
    }

    #[test]
    fn empty_and_whitespace_only_commands_are_rejected() {
        assert!(error("").contains("empty"));
        assert!(error("   \t\n ").contains("empty"));
        assert!(error(r#""""#).contains("empty program name"));
        assert!(error("''").contains("empty program name"));
    }

    #[test]
    fn a_placeholder_program_name_compiles() {
        // Whether it renders to anything usable is per-record, so it is checked
        // at preparation rather than rejected here.
        assert_eq!(render("{hostname} --flag"), ["alpha.local", "--flag"]);
    }

    #[test]
    fn fields_and_references_report_the_compiled_placeholders() {
        let template =
            CommandTemplate::compile("curl http://{hostname}:{port}/{txt.path}").unwrap();

        assert_eq!(
            template.fields().collect::<Vec<_>>(),
            ["hostname", "port", "txt.path"]
        );
        assert!(template.references("port"));
        assert!(!template.references("address"));
        // A literal brace is not a reference, which a raw `contains("{port}")`
        // check on the template text could not tell apart.
        assert!(
            !CommandTemplate::compile("echo {{port}")
                .unwrap()
                .references("port")
        );
    }

    #[test]
    fn a_missing_field_fails_rendering_rather_than_dropping_an_argument() {
        let record = Entry::new("alpha", "_ssh._tcp", "local");
        let template = CommandTemplate::compile("ssh {hostname}").unwrap();

        let err = template.render(&record).unwrap_err().to_string();

        assert!(err.contains("service field `hostname`"));
        assert!(err.contains("alpha"));
    }

    #[test]
    fn field_values_cannot_reshape_argv() {
        // Every hostile construct the grammar gives meaning to — separators,
        // both quote styles, escapes, and braces — arriving inside one field
        // value. All of it must land in exactly one argument, uninterpreted.
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some(r#"h.local' -oProxyCommand=evil ' "x" \ {name} {{ }"#.to_string());
        let template = CommandTemplate::compile("ssh {hostname} tail").unwrap();

        let argv = template.render(&record).unwrap();

        assert_eq!(
            argv,
            [
                "ssh",
                r#"h.local' -oProxyCommand=evil ' "x" \ {name} {{ }"#,
                "tail",
            ]
        );
    }
}
