use std::process::{Command, Stdio};

use color_eyre::eyre::{Result, eyre};

use crate::{
    plumber::{ActionMode, CommandAction},
    service::ServiceRecord,
};

#[derive(Debug, Clone)]
pub struct PreparedCommand {
    pub argv: Vec<String>,
    pub mode: ActionMode,
}

pub fn prepare(action: &CommandAction, record: &ServiceRecord) -> Result<PreparedCommand> {
    let expanded = interpolate(&action.command, record)?;
    let argv = split_command_line(&expanded)?;
    if argv.is_empty() {
        return Err(eyre!("action command expanded to an empty argv"));
    }
    Ok(PreparedCommand {
        argv,
        mode: action.mode,
    })
}

pub fn fork(command: &PreparedCommand) -> Result<()> {
    let mut child = Command::new(&command.argv[0]);
    child.args(&command.argv[1..]);
    child
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

pub fn exec(command: PreparedCommand) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let mut process = Command::new(&command.argv[0]);
        process.args(&command.argv[1..]);
        let err = process.exec();
        Err(err.into())
    }

    #[cfg(not(unix))]
    {
        let status = Command::new(&command.argv[0])
            .args(&command.argv[1..])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(eyre!("process exited with status {status}"))
        }
    }
}

fn interpolate(template: &str, record: &ServiceRecord) -> Result<String> {
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            output.push(ch);
            continue;
        }
        let mut field = String::new();
        loop {
            let Some(next) = chars.next() else {
                return Err(eyre!("unterminated interpolation in `{template}`"));
            };
            if next == '}' {
                break;
            }
            field.push(next);
        }
        let Some(value) = record.field(&field) else {
            return Err(eyre!(
                "service field `{field}` is unavailable for `{}`",
                record.name
            ));
        };
        output.push_str(&value);
    }
    Ok(output)
}

fn split_command_line(command: &str) -> Result<Vec<String>> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => quote = Some(ch),
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    argv.push(std::mem::take(&mut current));
                }
            }
            (None, c) => current.push(c),
        }
    }

    if let Some(q) = quote {
        return Err(eyre!("unterminated `{q}` quote in command"));
    }
    if !current.is_empty() {
        argv.push(current);
    }
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plumber::ActionMode;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn interpolates_and_splits() {
        let mut record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        let action = CommandAction {
            description: None,
            command: "ssh '{hostname}'".to_string(),
            mode: ActionMode::Execute,
        };
        let prepared = prepare(&action, &record).unwrap();
        assert_eq!(prepared.argv, vec!["ssh", "alpha.local"]);
    }

    #[test]
    fn prepares_all_supported_service_fields() {
        let mut record = ServiceRecord::new("Kitchen Printer", "_ipp._tcp", "local");
        record.hostname = Some("printer.local".to_string());
        record.address = Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 20)));
        record.port = Some(631);
        record
            .txt
            .insert("path".to_string(), "/ipp/print".to_string());
        let action = CommandAction {
            description: None,
            command: "open '{name}' {service_type} {domain} {hostname} {address} {port} {txt.path}"
                .to_string(),
            mode: ActionMode::Fork,
        };

        let prepared = prepare(&action, &record).unwrap();

        assert_eq!(prepared.mode, ActionMode::Fork);
        assert_eq!(
            prepared.argv,
            vec![
                "open",
                "Kitchen Printer",
                "_ipp._tcp",
                "local",
                "printer.local",
                "192.0.2.20",
                "631",
                "/ipp/print",
            ]
        );
    }

    #[test]
    fn splits_quoted_and_escaped_arguments() {
        let record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        let action = CommandAction {
            description: None,
            command: r#"printf "two words" one\ arg 'single quoted' "\\""#.to_string(),
            mode: ActionMode::Execute,
        };

        let prepared = prepare(&action, &record).unwrap();

        assert_eq!(
            prepared.argv,
            vec!["printf", "two words", "one arg", "single quoted", "\\"]
        );
    }

    #[test]
    fn missing_interpolation_field_is_an_error() {
        let record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        let action = CommandAction {
            description: None,
            command: "ssh {hostname}".to_string(),
            mode: ActionMode::Execute,
        };

        let err = prepare(&action, &record).unwrap_err();

        assert!(err.to_string().contains("service field `hostname`"));
        assert!(err.to_string().contains("alpha"));
    }

    #[test]
    fn malformed_templates_and_quotes_are_errors() {
        let record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        let unterminated_interpolation = CommandAction {
            description: None,
            command: "echo {name".to_string(),
            mode: ActionMode::Execute,
        };
        let unterminated_quote = CommandAction {
            description: None,
            command: "echo 'alpha".to_string(),
            mode: ActionMode::Execute,
        };

        assert!(
            prepare(&unterminated_interpolation, &record)
                .unwrap_err()
                .to_string()
                .contains("unterminated interpolation")
        );
        assert!(
            prepare(&unterminated_quote, &record)
                .unwrap_err()
                .to_string()
                .contains("unterminated `'` quote")
        );
    }

    #[test]
    fn fork_spawns_a_real_process() {
        let record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        let action = CommandAction {
            description: None,
            command: "true".to_string(),
            mode: ActionMode::Fork,
        };

        let prepared = prepare(&action, &record).unwrap();
        assert_eq!(prepared.mode, ActionMode::Fork);
        // `true` exits 0 immediately; forking it should succeed without error.
        fork(&prepared).unwrap();
    }

    #[test]
    fn fork_reports_a_missing_binary() {
        let command = PreparedCommand {
            argv: vec!["avahi-tui-no-such-binary-xyz".to_string()],
            mode: ActionMode::Fork,
        };

        assert!(fork(&command).is_err());
    }

    #[test]
    fn interpolates_txt_record_fields() {
        let mut record = ServiceRecord::new("nas", "_http._tcp", "local");
        record.hostname = Some("nas.local".to_string());
        record.txt.insert("path".to_string(), "/admin".to_string());
        let action = CommandAction {
            description: None,
            command: "xdg-open http://{hostname}{txt.path}".to_string(),
            mode: ActionMode::Fork,
        };

        let prepared = prepare(&action, &record).unwrap();

        assert_eq!(prepared.argv, vec!["xdg-open", "http://nas.local/admin"]);
    }

    #[test]
    fn missing_txt_field_is_an_error() {
        let record = ServiceRecord::new("nas", "_http._tcp", "local");
        let action = CommandAction {
            description: None,
            command: "echo {txt.path}".to_string(),
            mode: ActionMode::Fork,
        };

        let err = prepare(&action, &record).unwrap_err();

        assert!(err.to_string().contains("service field `txt.path`"));
    }

    #[test]
    fn empty_command_after_splitting_is_an_error() {
        let record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        let action = CommandAction {
            description: None,
            command: "   ".to_string(),
            mode: ActionMode::Execute,
        };

        let err = prepare(&action, &record).unwrap_err();

        assert!(err.to_string().contains("empty argv"));
    }
}
