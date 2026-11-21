use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Gui,
    Diagnose,
    Request(PathBuf),
    ApplyUpdate(String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Arguments {
    pub mode: Mode,
    pub workspace: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub trial: Option<String>,
    pub parent_pid: Option<u32>,
    pub wait: bool,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut result = Arguments {
        mode: Mode::Gui,
        workspace: None,
        output: None,
        trial: None,
        parent_pid: None,
        wait: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => {
                result.workspace = Some(PathBuf::from(
                    args.next().ok_or("--workspace requires a path")?,
                ))
            }
            "--output" => {
                result.output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a path")?,
                ))
            }
            "--wait" => result.wait = true,
            "--parent-pid" => {
                result.parent_pid = Some(
                    args.next()
                        .ok_or("--parent-pid requires a PID")?
                        .parse()
                        .map_err(|_| "Invalid parent PID")?,
                )
            }
            "--update-trial" => {
                result.trial = Some(update_id(
                    args.next().ok_or("--update-trial requires an ID")?,
                )?)
            }
            "--diagnose" | "--request-file" | "--apply-update" => {
                if result.mode != Mode::Gui {
                    return Err("Only one entry mode may be selected".into());
                }
                result.mode = match arg.as_str() {
                    "--diagnose" => Mode::Diagnose,
                    "--request-file" => Mode::Request(PathBuf::from(
                        args.next().ok_or("--request-file requires a path")?,
                    )),
                    _ => Mode::ApplyUpdate(update_id(
                        args.next().ok_or("--apply-update requires an ID")?,
                    )?),
                };
            }
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }
    if matches!(result.mode, Mode::ApplyUpdate(_)) && result.parent_pid.is_none() {
        return Err("--apply-update requires --parent-pid".into());
    }
    if result.trial.is_some() && result.mode != Mode::Gui {
        return Err("An update trial must render the desktop interface".into());
    }
    Ok(result)
}

pub fn update_id(value: String) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("Invalid managed update ID".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Result<Arguments, String> {
        parse(values.iter().map(|v| v.to_string()))
    }
    #[test]
    fn accepts_diagnosis_in_an_explicit_workspace() {
        let a = args(&[
            "--workspace",
            "E:/隔离 目录",
            "--diagnose",
            "--output",
            "result.json",
        ])
        .unwrap();
        assert_eq!(a.mode, Mode::Diagnose);
        assert_eq!(a.workspace, Some(PathBuf::from("E:/隔离 目录")));
        assert_eq!(a.output, Some(PathBuf::from("result.json")));
    }
    #[test]
    fn accepts_only_managed_update_ids_and_requires_parent() {
        assert!(args(&["--apply-update", "../../other", "--parent-pid", "1"]).is_err());
        assert!(args(&["--apply-update", "update-one"]).is_err());
        assert_eq!(
            args(&["--apply-update", "update-one", "--parent-pid", "42"])
                .unwrap()
                .mode,
            Mode::ApplyUpdate("update-one".into())
        );
    }
    #[test]
    fn rejects_unknown_or_conflicting_modes() {
        assert!(args(&["--execute", "evil"]).is_err());
        assert!(args(&["--diagnose", "--request-file", "request.json"]).is_err());
        assert!(args(&["--workspace"]).is_err());
    }
}
