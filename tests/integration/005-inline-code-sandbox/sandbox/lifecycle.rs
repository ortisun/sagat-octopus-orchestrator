//! A script is built, run, and leaves nothing behind.

use super::{build, result_of, run, sandbox};
use sagat_domain::scripts::Language;

#[tokio::test]
async fn a_typescript_script_runs_and_returns_its_result() {
    let sandbox = sandbox();
    let outcome = run(
        &sandbox,
        Language::TypeScript,
        "export default async function (input: { a: number; b: number }): Promise<{ sum: number }> {\n  console.log('adding', input.a, input.b);\n  return { sum: input.a + input.b };\n}\n",
        r#"{"a":2,"b":3}"#,
    )
    .await;
    assert_eq!(result_of(&outcome), r#"{"sum":5}"#, "log: {}", outcome.log);
    assert!(outcome.log.contains("adding 2 3"), "{}", outcome.log);
    assert!(outcome.denied.is_empty());
}

#[tokio::test]
async fn a_python_script_runs_and_returns_its_result() {
    let sandbox = sandbox();
    let outcome = run(
        &sandbox,
        Language::Python,
        "def run(input, octopus):\n    print('hello', input['name'])\n    return {'greeting': 'hi ' + input['name']}\n",
        r#"{"name":"ada"}"#,
    )
    .await;
    assert_eq!(
        result_of(&outcome),
        r#"{"greeting":"hi ada"}"#,
        "log: {}",
        outcome.log
    );
    assert!(outcome.log.contains("hello ada"), "{}", outcome.log);
}

#[tokio::test]
async fn a_script_that_does_not_compile_is_rejected_at_build_time_with_the_reason() {
    let sandbox = sandbox();
    let ts = build(
        &sandbox,
        Language::TypeScript,
        "export default function ( {\n",
    )
    .await;
    assert!(
        matches!(
            ts,
            Err(sagat_application::sandbox::BuildFailure::Rejected(_))
        ),
        "{ts:?}"
    );
    let py = build(
        &sandbox,
        Language::Python,
        "def run(input, octopus:\n  return 1\n",
    )
    .await;
    assert!(
        matches!(
            py,
            Err(sagat_application::sandbox::BuildFailure::Rejected(_))
        ),
        "{py:?}"
    );
    let missing = build(&sandbox, Language::Python, "x = 1\n").await;
    assert!(
        matches!(
            missing,
            Err(sagat_application::sandbox::BuildFailure::Rejected(_))
        ),
        "{missing:?}"
    );
}

#[tokio::test]
async fn the_same_source_builds_to_the_same_artifact_bytes() {
    let sandbox = sandbox();
    let source = "export default (input: unknown) => ({ echo: input });\n";
    let a = build(&sandbox, Language::TypeScript, source)
        .await
        .expect("first build");
    let b = build(&sandbox, Language::TypeScript, source)
        .await
        .expect("second build");
    assert_eq!(a.artifact, b.artifact);
    assert_eq!(a.toolchain, b.toolchain);
}

#[tokio::test]
async fn a_script_that_throws_fails_with_its_message_and_the_run_is_cleaned_up() {
    let sandbox = sandbox();
    let outcome = run(
        &sandbox,
        Language::TypeScript,
        "export default function () { throw new Error('boom'); }\n",
        "{}",
    )
    .await;
    assert_eq!(result_of(&outcome), "FAILED SCRIPT_ERROR");
    assert!(outcome.log.contains("boom"), "{}", outcome.log);
    assert_eq!(
        sandbox.leftovers().await,
        0,
        "nothing left behind by the run"
    );
}

#[tokio::test]
async fn every_way_of_exporting_the_entry_point_the_runner_loads_builds_and_a_comment_is_not_one() {
    let sandbox = sandbox();
    for source in [
        "export const run = async (input: unknown) => ({ via: 'const' });\n",
        "function main() { return { via: 'list' }; }\nexport { main as default };\n",
        "export async function run() { return { via: 'function' }; }\n",
    ] {
        let outcome = run(&sandbox, Language::TypeScript, source, "{}").await;
        assert!(
            result_of(&outcome).contains(r#""via":"#),
            "{source}: {} / {}",
            result_of(&outcome),
            outcome.log
        );
    }
    let only_a_comment = build(
        &sandbox,
        Language::TypeScript,
        "// export default function () {}\nconst x = 1;\n",
    )
    .await;
    assert!(
        matches!(
            only_a_comment,
            Err(sagat_application::sandbox::BuildFailure::Rejected(_))
        ),
        "{only_a_comment:?}"
    );
}
