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
    DiffSession, render_dot_timeline, render_mermaid_timeline, render_text_timeline,
};

const I64: TypeDescriptor = TypeDescriptor::new("i64");
const STRING: TypeDescriptor = TypeDescriptor::new("String");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Text,
    Mermaid,
    Dot,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    Basic,
    SameOutput,
    Conditional,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CliOptions {
    format: OutputFormat,
    scenario: Scenario,
}

#[derive(Debug)]
enum CliError {
    Graph(GraphError),
    MissingLatestSnapshot,
    UnknownArgument(String),
    UnknownFormat(String),
    UnknownScenario(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(error) => write!(f, "{error}"),
            Self::MissingLatestSnapshot => write!(f, "diff session did not produce a snapshot"),
            Self::UnknownArgument(argument) => write!(f, "unknown argument {argument:?}"),
            Self::UnknownFormat(format) => write!(f, "unknown format {format:?}"),
            Self::UnknownScenario(scenario) => write!(f, "unknown scenario {scenario:?}"),
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

#[derive(Debug)]
struct CappedRule {
    input: Attribute<i64>,
    cap: i64,
}

#[derive(Debug)]
struct LabelRule {
    input: Attribute<i64>,
}

#[derive(Debug)]
struct ConditionalPriceRule {
    use_sale_price: Attribute<bool>,
    sale_price: Attribute<i64>,
    regular_price: Attribute<i64>,
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
    let Some(options) = parse_args()? else {
        println!("{}", usage());
        return Ok(());
    };

    let session = run_scenario(options.scenario)?;
    ensure_session_has_snapshots(&session)?;

    match options.format {
        OutputFormat::Text => {
            print!("{}", render_text_timeline(&session));
        }
        OutputFormat::Mermaid => {
            print!("{}", render_mermaid_timeline(&session));
        }
        OutputFormat::Dot => {
            print!("{}", render_dot_timeline(&session));
        }
        OutputFormat::All => {
            println!("# Scenario: {}", scenario_name(options.scenario));
            println!();
            println!("# Text Timeline");
            println!("{}", render_text_timeline(&session));
            println!("# Mermaid Timeline");
            println!("```mermaid");
            print!("{}", render_mermaid_timeline(&session));
            println!("```");
            println!("# Graphviz DOT Timeline");
            println!("```dot");
            print!("{}", render_dot_timeline(&session));
            println!("```");
        }
    }

    Ok(())
}

fn ensure_session_has_snapshots(session: &DiffSession) -> Result<(), CliError> {
    session
        .latest_snapshot()
        .map(|_| ())
        .ok_or(CliError::MissingLatestSnapshot)
}

fn parse_args() -> Result<Option<CliOptions>, CliError> {
    let mut args = env::args().skip(1);
    let mut options = CliOptions {
        format: OutputFormat::All,
        scenario: Scenario::Basic,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--format" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::UnknownFormat("<missing>".to_string()))?;
                options.format = parse_format(&value)?;
            }
            value if value.starts_with("--format=") => {
                let value = value.trim_start_matches("--format=");
                options.format = parse_format(value)?;
            }
            "--scenario" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::UnknownScenario("<missing>".to_string()))?;
                options.scenario = parse_scenario(&value)?;
            }
            value if value.starts_with("--scenario=") => {
                let value = value.trim_start_matches("--scenario=");
                options.scenario = parse_scenario(value)?;
            }
            _ => return Err(CliError::UnknownArgument(arg)),
        }
    }

    Ok(Some(options))
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

fn parse_scenario(value: &str) -> Result<Scenario, CliError> {
    match value {
        "basic" => Ok(Scenario::Basic),
        "same-output" => Ok(Scenario::SameOutput),
        "conditional" => Ok(Scenario::Conditional),
        _ => Err(CliError::UnknownScenario(value.to_string())),
    }
}

fn scenario_name(scenario: Scenario) -> &'static str {
    match scenario {
        Scenario::Basic => "basic",
        Scenario::SameOutput => "same-output",
        Scenario::Conditional => "conditional",
    }
}

fn usage() -> &'static str {
    "Usage: cargo run --manifest-path diff/Cargo.toml -- [--scenario basic|same-output|conditional] [--format text|mermaid|dot|all]\n\
     \n\
     Runs a built-in AttributeGraph scenario and prints graph diffs for visual debugging.\n\
     Default scenario: basic\n\
     Default format: all"
}

fn run_scenario(scenario: Scenario) -> Result<DiffSession, GraphError> {
    match scenario {
        Scenario::Basic => run_basic_scenario(),
        Scenario::SameOutput => run_same_output_scenario(),
        Scenario::Conditional => run_conditional_scenario(),
    }
}

fn run_basic_scenario() -> Result<DiffSession, GraphError> {
    let mut graph = AttributeGraph::new();
    let mut session = DiffSession::new();

    session.capture("empty graph", &graph)?;

    let price = graph.add_static_attribute(10_i64);
    let quantity = graph.add_static_attribute(2_i64);
    let shipping = graph.add_static_attribute(3_i64);
    session.label_attribute(price.attribute(), "price");
    session.label_attribute(quantity.attribute(), "quantity");
    session.label_attribute(shipping.attribute(), "shipping");

    let total = graph.add_dynamic_attribute::<i64>(boxed_rule(
        SumRule {
            lhs: price.attribute(),
            rhs: quantity.attribute(),
        },
        update_sum,
        I64,
        "price + quantity",
    ))?;
    session.label_attribute(total.attribute(), "total");

    let grand_total = graph.add_dynamic_attribute::<i64>(boxed_rule(
        SumRule {
            lhs: total.attribute(),
            rhs: shipping.attribute(),
        },
        update_sum,
        I64,
        "total + shipping",
    ))?;
    session.label_attribute(grand_total.attribute(), "grand total");
    session.capture("created attributes", &graph)?;

    let _ = graph.read(grand_total)?;
    session.capture("read grand total", &graph)?;

    graph.set_static(price, 11)?;
    session.capture("price changed", &graph)?;

    let _ = graph.read(grand_total)?;
    session.capture("read grand total again", &graph)?;

    Ok(session)
}

fn run_same_output_scenario() -> Result<DiffSession, GraphError> {
    let mut graph = AttributeGraph::new();
    let mut session = DiffSession::new();

    session.capture("empty graph", &graph)?;

    let price = graph.add_static_attribute(12_i64);
    let shipping = graph.add_static_attribute(5_i64);
    session.label_attribute(price.attribute(), "price");
    session.label_attribute(shipping.attribute(), "shipping");

    let capped_price = graph.add_dynamic_attribute::<i64>(boxed_rule(
        CappedRule {
            input: price.attribute(),
            cap: 10,
        },
        update_capped,
        I64,
        "min(price, 10)",
    ))?;
    session.label_attribute(capped_price.attribute(), "capped price");

    let price_label = graph.add_dynamic_attribute::<String>(boxed_rule(
        LabelRule {
            input: capped_price.attribute(),
        },
        update_label,
        STRING,
        "label capped price",
    ))?;
    session.label_attribute(price_label.attribute(), "price label");

    let capped_total = graph.add_dynamic_attribute::<i64>(boxed_rule(
        SumRule {
            lhs: capped_price.attribute(),
            rhs: shipping.attribute(),
        },
        update_sum,
        I64,
        "capped price + shipping",
    ))?;
    session.label_attribute(capped_total.attribute(), "capped total");
    session.capture("created attributes", &graph)?;

    let _ = graph.read(capped_total)?;
    let _ = graph.read(price_label)?;
    session.capture("read dependents", &graph)?;

    graph.set_static(price, 13)?;
    session.capture("price changed above cap", &graph)?;

    let _ = graph.read(capped_total)?;
    let _ = graph.read(price_label)?;
    session.capture("read dependents again", &graph)?;

    Ok(session)
}

fn run_conditional_scenario() -> Result<DiffSession, GraphError> {
    let mut graph = AttributeGraph::new();
    let mut session = DiffSession::new();

    session.capture("empty graph", &graph)?;

    let use_sale_price = graph.add_static_attribute(true);
    let sale_price = graph.add_static_attribute(7_i64);
    let regular_price = graph.add_static_attribute(10_i64);
    session.label_attribute(use_sale_price.attribute(), "use sale price");
    session.label_attribute(sale_price.attribute(), "sale price");
    session.label_attribute(regular_price.attribute(), "regular price");

    let selected_price = graph.add_dynamic_attribute::<i64>(boxed_rule(
        ConditionalPriceRule {
            use_sale_price: use_sale_price.attribute(),
            sale_price: sale_price.attribute(),
            regular_price: regular_price.attribute(),
        },
        update_conditional_price,
        I64,
        "selected price",
    ))?;
    session.label_attribute(selected_price.attribute(), "selected price");
    session.capture("created attributes", &graph)?;

    let _ = graph.read(selected_price)?;
    session.capture("read selected price", &graph)?;

    graph.set_static(use_sale_price, false)?;
    session.capture("switched to regular price", &graph)?;

    let _ = graph.read(selected_price)?;
    session.capture("read selected after switch", &graph)?;

    graph.set_static(sale_price, 6)?;
    session.capture("inactive sale price changed", &graph)?;

    graph.set_static(regular_price, 11)?;
    session.capture("active regular price changed", &graph)?;

    let _ = graph.read(selected_price)?;
    session.capture("read selected again", &graph)?;

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

fn update_capped(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<CappedRule>(handle);
    let value = context.read_attribute(rule.input)?;

    context.set_output_value(value.min(rule.cap));
    Ok(())
}

fn update_label(handle: RuleHandle, context: &mut EvaluationContext<'_>) -> Result<(), GraphError> {
    let rule = rule_body::<LabelRule>(handle);
    let value = context.read_attribute(rule.input)?;
    let label = if value == 10 { "capped" } else { "other" };

    context.set_output_value(label.to_string());
    Ok(())
}

fn update_conditional_price(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<ConditionalPriceRule>(handle);
    let selected_price = if context.read_attribute(rule.use_sale_price)? {
        context.read_attribute(rule.sale_price)?
    } else {
        context.read_attribute(rule.regular_price)?
    };

    context.set_output_value(selected_price);
    Ok(())
}
