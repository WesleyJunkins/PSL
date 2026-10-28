//! `psl run`.

use super::{pipeline_path, workspace_root, Args, CliError, CliResult};
use crate::check::check_pipeline;
use crate::json::Json;
use crate::supervisor::proc::{python_for, sdk_path};
use crate::supervisor::{LogLevel, RunOptions};

pub fn run(args: &Args) -> CliResult {
    let root = workspace_root(args)?;
    let path = pipeline_path(&root, args);
    let r = check_pipeline(&root, &path);
    eprint!("{}", r.diags.render(&r.sm));
    if r.diags.has_errors() {
        eprintln!(
            "run: the pipeline has {} error{}; fix them first",
            r.diags.error_count(),
            if r.diags.error_count() == 1 { "" } else { "s" }
        );
        return Ok(1);
    }
    let mut graph = r.graph.unwrap();
    // --param node.name=value overrides, validated against the manifest.
    for p in args.values("--param") {
        let (target, raw) = p.split_once('=').ok_or_else(|| {
            CliError::usage(format!("--param expects node.name=value, got `{}`", p))
        })?;
        let (node, name) = target.split_once('.').ok_or_else(|| {
            CliError::usage(format!("--param expects node.name=value, got `{}`", p))
        })?;
        let value = parse_value(raw);
        let n = graph
            .nodes
            .iter_mut()
            .find(|n| n.id == node)
            .ok_or_else(|| CliError::usage(format!("--param: no node `{}`", node)))?;
        let decl = n.manifest.param(name).ok_or_else(|| {
            CliError::usage(format!("--param: `{}` has no parameter `{}`", node, name))
        })?;
        if let Err(e) = crate::model::manifest::check_param_value(decl, &value) {
            return Err(CliError::usage(format!(
                "--param: parameter `{}` of `{}` {}",
                name,
                node,
                e.message()
            )));
        }
        n.params.set(name, value);
    }
    let sdk = sdk_path(&root);
    let setup_version = std::fs::read_to_string(root.join(".psl").join("version"))
        .map(|v| v.trim().to_string())
        .ok();
    if std::env::var("PSL_SDK_PATH").is_err() && setup_version.as_deref() != Some(crate::VERSION) {
        eprintln!(
            "warning: this workspace was set up with psl {}; this is psl {}. Run `psl init` here to refresh the packages and SDK.",
            setup_version.unwrap_or_else(|| "an older version".into()),
            crate::VERSION
        );
    }
    if graph
        .nodes
        .iter()
        .any(|n| matches!(n.manifest.runtime, crate::model::Runtime::Python { .. }))
        && !sdk.join("psl").is_dir()
    {
        return Err(CliError::failed(format!(
            "the Python SDK is not installed in this workspace ({} is missing); run `psl init` here or set PSL_SDK_PATH",
            sdk.display()
        )));
    }
    let python = python_for(
        &root,
        r.workspace.as_ref().and_then(|w| w.python.as_deref()),
    );
    let echo_level = if args.any_flag(&["-q", "--quiet"]) {
        LogLevel::Warn
    } else if args.any_flag(&["-v", "--verbose"]) {
        LogLevel::Debug
    } else {
        LogLevel::Info
    };
    let record = args
        .value("--record")
        .map(std::path::PathBuf::from)
        .or_else(|| graph.runtime.record.clone().map(std::path::PathBuf::from));
    let opts = RunOptions {
        record,
        echo_level,
        python,
        sdk_path: sdk,
    };
    let code = crate::supervisor::run(&root, graph, opts)?;
    Ok(code)
}

/// Parse a command-line value: JSON if it parses, else a string.
pub fn parse_value(raw: &str) -> Json {
    match Json::parse(raw) {
        Ok(j) => j,
        Err(_) => Json::Str(raw.to_string()),
    }
}
