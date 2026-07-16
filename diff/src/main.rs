use std::any::type_name;
use std::env;
use std::error::Error;
use std::fmt;
use std::process::ExitCode;

use attribute_graph::{
    Attribute, AttributeGraph, EvaluationContext, GraphError, RuleDescriptor, RuleHandle,
    TypeDescriptor, UpdateFn,
};
use attribute_graph_diff::{
    DiffSession, render_dot_snapshot, render_mermaid_snapshot, render_text_timeline,
};

const I64: TypeDescriptor = TypeDescriptor::new("i64");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Text,
    Mermaid,
    Dot,
    All,
}

#[derive(Debug)]
enum CliError {
    Graph(GraphError),
    MissingLatestSnapshot,
    UnknownArgument(String),
    UnknownFormat(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(error) => write!(f, "{error}"),
            Self::MissingLatestSnapshot => write!(f, "diff session did not produce a snapshot"),
            Self::UnknownArgument(argument) => write!(f, "unknown argument {argument:?}"),
            Self::UnknownFormat(format) => write!(f, "unknown format {format:?}"),
        }
    }
}

impl Error for CliError {}

impl From<GraphError> for CliError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

#[derive(Debug)]
struct SumRule {
    lhs: Attribute<i64>,
    rhs: Attribute<i64>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("attribute_graph_diff: {error}");
            eprintln!();
            eprintln!("{}", usage());
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), CliError> {
    let Some(format) = parse_args()? else {
        println!("{}", usage());
        return Ok(());
    };

    let session = run_demo_scenario()?;
    let latest_snapshot = session
        .latest_snapshot()
        .ok_or(CliError::MissingLatestSnapshot)?;

    match format {
        OutputFormat::Text => {
            print!("{}", render_text_timeline(&session));
        }
        OutputFormat::Mermaid => {
            print!("{}", render_mermaid_snapshot(latest_snapshot));
        }
        OutputFormat::Dot => {
            print!("{}", render_dot_snapshot(latest_snapshot));
        }
        OutputFormat::All => {
            println!("# Text Timeline");
            println!("{}", render_text_timeline(&session));
            println!("# Final Mermaid Snapshot");
            println!("```mermaid");
            print!("{}", render_mermaid_snapshot(latest_snapshot));
            println!("```");
            println!("# Final Graphviz DOT Snapshot");
            println!("```dot");
            print!("{}", render_dot_snapshot(latest_snapshot));
            println!("```");
        }
    }

    Ok(())
}

fn parse_args() -> Result<Option<OutputFormat>, CliError> {
    let mut args = env::args().skip(1);
    let mut format = OutputFormat::All;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--format" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::UnknownFormat("<missing>".to_string()))?;
                format = parse_format(&value)?;
            }
            value if value.starts_with("--format=") => {
                let value = value.trim_start_matches("--format=");
                format = parse_format(value)?;
            }
            _ => return Err(CliError::UnknownArgument(arg)),
        }
    }

    Ok(Some(format))
}

fn parse_format(value: &str) -> Result<OutputFormat, CliError> {
    match value {
        "text" => Ok(OutputFormat::Text),
        "mermaid" => Ok(OutputFormat::Mermaid),
        "dot" => Ok(OutputFormat::Dot),
        "all" => Ok(OutputFormat::All),
        _ => Err(CliError::UnknownFormat(value.to_string())),
    }
}

fn usage() -> &'static str {
    "Usage: cargo run --manifest-path diff/Cargo.toml -- [--format text|mermaid|dot|all]\n\
     \n\
     Runs a built-in AttributeGraph scenario and prints graph diffs for visual debugging.\n\
     Default format: all"
}

fn run_demo_scenario() -> Result<DiffSession, GraphError> {
    let mut graph = AttributeGraph::new();
    let mut session = DiffSession::new();

    session.capture("empty graph", &graph)?;

    let price = graph.add_static_attribute(10_i64);
    let quantity = graph.add_static_attribute(2_i64);
    let multiplier = graph.add_static_attribute(3_i64);
    let total = graph.add_dynamic_attribute::<i64>(boxed_rule(
        SumRule {
            lhs: price.attribute(),
            rhs: quantity.attribute(),
        },
        update_sum,
        I64,
        "price + quantity",
    ))?;
    let scaled_total = graph.add_dynamic_attribute::<i64>(boxed_rule(
        SumRule {
            lhs: total.attribute(),
            rhs: multiplier.attribute(),
        },
        update_sum,
        I64,
        "total + multiplier",
    ))?;
    session.capture("created attributes", &graph)?;

    let _ = graph.read(scaled_total)?;
    session.capture("read scaled total", &graph)?;

    graph.set_static(price, 11)?;
    session.capture("price changed", &graph)?;

    let _ = graph.read(scaled_total)?;
    session.capture("read scaled total again", &graph)?;

    Ok(session)
}

fn boxed_rule<T: 'static>(
    body: T,
    update: UpdateFn,
    value_type: TypeDescriptor,
    debug_name: &'static str,
) -> RuleDescriptor {
    let body = Box::new(body);
    let handle = RuleHandle::from_raw(Box::into_raw(body) as usize);

    RuleDescriptor::new(
        handle,
        update,
        TypeDescriptor::new(type_name::<T>()),
        value_type,
        debug_name,
    )
    .with_destroy(drop_boxed_rule::<T>)
}

fn drop_boxed_rule<T>(handle: RuleHandle) {
    unsafe {
        drop(Box::from_raw(handle.raw() as *mut T));
    }
}

fn rule_body<T>(handle: RuleHandle) -> &'static T {
    unsafe { &*(handle.raw() as *const T) }
}

fn update_sum(handle: RuleHandle, context: &mut EvaluationContext<'_>) -> Result<(), GraphError> {
    let rule = rule_body::<SumRule>(handle);
    let lhs = context.read_attribute(rule.lhs)?;
    let rhs = context.read_attribute(rule.rhs)?;

    context.set_output_value(lhs + rhs);
    Ok(())
}
