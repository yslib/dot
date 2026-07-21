use std::fmt;
use std::fmt::Write as _;

use crate::plan::ExecutionPlan;
use crate::schema::{
    Action, EnvironmentPatch, ExecAction, LinkConflict, LinkMissingParent, OneOrMany,
};

pub const fn display(plan: &ExecutionPlan) -> DryRunDisplay<'_> {
    DryRunDisplay { plan }
}

#[derive(Clone, Copy, Debug)]
pub struct DryRunDisplay<'a> {
    plan: &'a ExecutionPlan,
}

impl fmt::Display for DryRunDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plan = self.plan;
        let platform = plan.platform();
        let mut output = String::new();
        writeln!(output, "target: {}", plan.target())?;
        writeln!(output, "profile: {}", plan.profile().unwrap_or("<root>"))?;
        writeln!(output, "platform: {}/{}", platform.os, platform.arch)?;
        if let Some(distro) = &platform.distro {
            writeln!(output, "distro: {distro}")?;
        }
        if !platform.distro_families.is_empty() {
            writeln!(output, "distro families: {:?}", platform.distro_families)?;
        }
        writeln!(output, "environments: {:?}", platform.environments)?;

        writeln!(output, "\nproviders:")?;
        if plan.providers().is_empty() {
            writeln!(output, "  <none>")?;
        }
        for provider in plan.providers() {
            writeln!(output, "  {}", provider.id())?;
            match provider.activate() {
                Some(activate) => {
                    writeln!(output, "    activate:")?;
                    write_environment_patch(&mut output, activate, "      ")?;
                }
                None => writeln!(output, "    activate: <none>")?,
            }
            write_exec_action(&mut output, "probe", provider.probe(), "    ")?;
            if provider.ensure().is_empty() {
                writeln!(output, "    ensure: <none>")?;
            } else {
                writeln!(output, "    ensure:")?;
                for (index, action) in provider.ensure().iter().enumerate() {
                    writeln!(output, "      [{index}]")?;
                    write_exec_fields(&mut output, action, "        ")?;
                }
            }
        }

        writeln!(output, "\nprovider packages:")?;
        if plan.provider_batches().is_empty() {
            writeln!(output, "  <none>")?;
        }
        for batch in plan.provider_batches() {
            writeln!(output, "  {}", batch.provider())?;
            writeln!(output, "    provider_args: {:?}", batch.provider_args())?;
            writeln!(output, "    packages: {:?}", batch.packages())?;
            write_exec_action(&mut output, "install", batch.install(), "    ")?;
        }

        writeln!(output, "\nmanual packages:")?;
        if plan.manual_packages().is_empty() {
            writeln!(output, "  <none>")?;
        }
        for package in plan.manual_packages() {
            writeln!(output, "  {}", package.id())?;
            write_action(&mut output, package.install(), "    ")?;
        }

        writeln!(output, "\nactions:")?;
        if plan.actions().is_empty() {
            writeln!(output, "  <none>")?;
        }
        for action in plan.actions() {
            writeln!(output, "  {}", action.id())?;
            write_action(&mut output, action.action(), "    ")?;
        }

        writeln!(output, "\nlinks:")?;
        if plan.links().is_empty() {
            writeln!(output, "  <none>")?;
        }
        for link in plan.links() {
            writeln!(
                output,
                "  {}: {:?} -> {:?}",
                link.id(),
                link.source().display().to_string(),
                link.target().display().to_string()
            )?;
            writeln!(
                output,
                "    on_conflict: {}",
                link_conflict_name(link.on_conflict())
            )?;
            writeln!(
                output,
                "    on_missing_parent: {}",
                link_missing_parent_name(link.on_missing_parent())
            )?;
        }

        formatter.write_str(output.trim_end())
    }
}

fn write_action(output: &mut String, action: &Action, indent: &str) -> fmt::Result {
    match &action.check {
        Some(check) => write_exec_action(output, "check", check, indent)?,
        None => writeln!(output, "{indent}check: <none>")?,
    }
    write_exec_action(output, "exec", &action.exec, indent)
}

fn write_exec_action(
    output: &mut String,
    label: &str,
    action: &ExecAction,
    indent: &str,
) -> fmt::Result {
    writeln!(output, "{indent}{label}:")?;
    let field_indent = format!("{indent}  ");
    write_exec_fields(output, action, &field_indent)
}

fn write_exec_fields(output: &mut String, action: &ExecAction, indent: &str) -> fmt::Result {
    writeln!(output, "{indent}program: {:?}", action.program.as_str())?;
    let args = action
        .args
        .iter()
        .map(|argument| argument.as_str())
        .collect::<Vec<_>>();
    writeln!(output, "{indent}args: {args:?}")?;
    if let Some(cwd) = &action.cwd {
        writeln!(output, "{indent}cwd: {:?}", cwd.as_str())?;
    }
    if let Some(environment) = &action.env {
        writeln!(output, "{indent}env:")?;
        let environment_indent = format!("{indent}  ");
        write_environment_patch(output, environment, &environment_indent)?;
    }
    Ok(())
}

fn write_environment_patch(
    output: &mut String,
    patch: &EnvironmentPatch,
    indent: &str,
) -> fmt::Result {
    if let Some(values) = &patch.path_prepend {
        writeln!(output, "{indent}path_prepend: {:?}", scalar_values(values))?;
    }
    if let Some(values) = &patch.path_append {
        writeln!(output, "{indent}path_append: {:?}", scalar_values(values))?;
    }
    if !patch.variables.is_empty() {
        writeln!(output, "{indent}variables:")?;
        for (name, value) in &patch.variables {
            writeln!(output, "{indent}  {name}: {:?}", value.as_str())?;
        }
    }
    Ok(())
}

fn scalar_values(values: &OneOrMany<crate::schema::ScalarTemplate>) -> Vec<&str> {
    match values {
        OneOrMany::One(value) => vec![value.as_str()],
        OneOrMany::Many(values) => values.iter().map(|value| value.as_str()).collect(),
    }
}

fn link_conflict_name(value: LinkConflict) -> &'static str {
    match value {
        LinkConflict::Error => "error",
        LinkConflict::ReplaceLink => "replace-link",
    }
}

fn link_missing_parent_name(value: LinkMissingParent) -> &'static str {
    match value {
        LinkMissingParent::Create => "create",
        LinkMissingParent::Skip => "skip",
    }
}
