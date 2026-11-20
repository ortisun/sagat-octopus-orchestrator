use super::*;
#[derive(Clone)]
pub struct ApprovalDecisionRequest {
    pub decision_key: String,
    pub decision: crate::ApprovalDecision,
    pub reason_code: Option<String>,
}
impl std::fmt::Debug for ApprovalDecisionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalDecisionRequest")
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}
fn request(run: &str, page: &PageOptions) -> Result<WireRequest, SagatError> {
    uuid(run)?;
    Ok(
        WireRequest::new(Method::GET, &["runs", run, "approvals"], 200)
            .query(page_query(page.limit, page.cursor.as_deref())?),
    )
}
fn parse(raw: Value, cursor: Option<&str>) -> Result<crate::ApprovalPage, SagatError> {
    if raw.get("nextCursor").is_none() {
        return Err(protocol());
    }
    let page = crate::types::parse_page(raw, crate::ApprovalListItem::parse)?;
    if page.next_cursor.is_some() && page.next_cursor.as_deref() == cursor {
        return Err(protocol());
    }
    Ok(page)
}
impl Client {
    pub async fn list_approvals(
        &self,
        run: &str,
        page: PageOptions,
        call: CallOptions,
    ) -> Result<crate::ApprovalPage, SagatError> {
        self.execute(request(run, &page)?, call)
            .await?
            .parse(|raw| parse(raw, page.cursor.as_deref()))
    }
    pub fn list_all_approvals(
        &self,
        run: &str,
        page: PageOptions,
        call: CallOptions,
    ) -> impl futures_util::Stream<Item = Result<crate::ApprovalListItem, SagatError>> + Send + 'static
    {
        futures_util::stream::try_unfold(
            (
                self.clone(),
                run.to_owned(),
                page,
                call,
                std::collections::VecDeque::<crate::ApprovalListItem>::new(),
                false,
            ),
            |(client, run, mut page, call, mut buffer, mut finished)| async move {
                loop {
                    if call.cancel.is_cancelled() {
                        return Err(cancelled());
                    }
                    if let Some(item) = buffer.pop_front() {
                        return Ok(Some((item, (client, run, page, call, buffer, finished))));
                    }
                    if finished {
                        return Ok(None);
                    }
                    let response = client
                        .list_approvals(&run, page.clone(), call.clone())
                        .await?;
                    finished = response.next_cursor.is_none();
                    page.cursor = response.next_cursor;
                    buffer = response.items.into();
                }
            },
        )
    }
    pub async fn decide_approval(
        &self,
        id: &str,
        decision: ApprovalDecisionRequest,
        call: CallOptions,
    ) -> Result<crate::ApprovalDecisionReceipt, SagatError> {
        uuid(id)?;
        if !body_key(&decision.decision_key, 128)
            || decision.reason_code.as_deref().is_some_and(|v| {
                v.is_empty()
                    || v.len() > 64
                    || !v.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-')
                    })
            })
        {
            return Err(validation());
        }
        let mut body = json!({"decisionKey":decision.decision_key,"decision":decision.decision});
        if let Some(reason) = decision.reason_code {
            body["reasonCode"] = json!(reason);
        }
        self.execute(
            WireRequest::new(Method::POST, &["approvals", id, "decisions"], 200)
                .json(&body)?
                .repeatable(),
            call,
        )
        .await?
        .parse(crate::ApprovalDecisionReceipt::parse)
    }
}
