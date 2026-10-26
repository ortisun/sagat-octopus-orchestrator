use crate::ExecutionError;
use sagat_domain::database::OperationKind;
use sqlparser::{
    ast::{Query, Select, Statement, Visit, Visitor},
    dialect::PostgreSqlDialect,
    parser::Parser,
};
use std::ops::ControlFlow;

/// Conservative language subset, backed by READ ONLY and target-role permissions at execution.
/// This is not a sandbox for functions/extensions installed in the customer's database.
pub fn validate_statement(sql: &str, kind: OperationKind) -> Result<(), ExecutionError> {
    if sql.len() > 64 * 1024 {
        return Err(ExecutionError::StatementDenied);
    }
    let ast = Parser::new(&PostgreSqlDialect {})
        .with_recursion_limit(64)
        .try_with_sql(sql)
        .and_then(|mut p| p.parse_statements())
        .map_err(|_| ExecutionError::StatementDenied)?;
    if ast.len() != 1 {
        return Err(ExecutionError::StatementDenied);
    }
    let allowed = match kind {
        OperationKind::Query => matches!(ast[0], Statement::Query(_)),
        OperationKind::Execute => matches!(
            ast[0],
            Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_)
        ),
    };
    if !allowed || ast.visit(&mut Guard { kind }).is_break() {
        return Err(ExecutionError::StatementDenied);
    }
    Ok(())
}
struct Guard {
    kind: OperationKind,
}
impl Visitor for Guard {
    type Break = ();
    fn pre_visit_statement(&mut self, stmt: &Statement) -> ControlFlow<()> {
        match stmt {
            Statement::Query(_) => ControlFlow::Continue(()),
            Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_)
                if self.kind == OperationKind::Execute =>
            {
                ControlFlow::Continue(())
            }
            _ => ControlFlow::Break(()),
        }
    }
    fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<()> {
        if select.into.is_some() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        if !query.locks.is_empty()
            || !query.pipe_operators.is_empty()
            || query.settings.is_some()
            || query.format_clause.is_some()
            || query.for_clause.is_some()
        {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}
