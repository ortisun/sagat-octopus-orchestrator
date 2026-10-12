//! The SQS consumer (`FR-004-005`, US03).
//!
//! Requests are made with the Query protocol, which every SQS-compatible service speaks, through the
//! guarded egress client, so the queue is a destination like any other and the namespace's policy
//! applies. A message is deleted from the queue only after the event it became is in the inbox; one
//! that is not deleted is delivered again when its visibility timeout ends, and the inbox recognises
//! the repeat.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aws_credential_types::Credentials as AwsCredentials;
use aws_sigv4::http_request::{SignableBody, SignableRequest, SigningSettings, sign};
use aws_sigv4::sign::v4;
use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::Method;
use sagat_application::connectors::{ConnectorRuntime, RawMessage};
use sagat_domain::connectors::ConnectorSettings;
use sagat_infra_egress::{EgressClient, EgressRequest, ResponseBody};
use tokio::sync::watch;

use crate::credentials::Credentials;
use crate::ingest::{Ingest, Outcome, Pacer};
use crate::nats::Stopped;

/// The most one answer of the queue is read to be: ten messages of 256 KiB, each as XML in which
/// every quotation mark is six characters. A batch of messages far smaller than this is what the
/// webhook client's limit would have refused, so SQS reads with a limit of its own.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Messages asked for per request (the most SQS gives).
const BATCH: u32 = 10;
/// Seconds a request waits for a message when the queue is empty.
const WAIT_SECONDS: u32 = 20;
/// Seconds a message is hidden from other consumers while this one decides about it.
const VISIBILITY_SECONDS: u32 = 60;
/// How long to wait before asking again after a failure.
const PAUSE: Duration = Duration::from_secs(5);
/// Consecutive failed calls to the queue after which the reader gives up and is started again. A
/// reader reads its credentials and the egress policy of its namespace once, when it starts; one
/// that only ever retried would go on with a key that has expired.
const RESTART_AFTER: u32 = 5;

/// How long the reader waits, and how many failures in a row it puts up with.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// The wait after a failed call, and the least wait after messages were left in the queue.
    pub pause: Duration,
    /// Failed calls in a row after which [`run_with`] returns [`Stopped::Failed`].
    pub restart_after: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            pause: PAUSE,
            restart_after: RESTART_AFTER,
        }
    }
}

/// Failed calls in a row.
#[derive(Debug, Default)]
struct Streak(u32);

impl Streak {
    /// Counts a failure; `true` when it was the one that makes the reader give up.
    fn failed(&mut self, limit: u32) -> bool {
        self.0 += 1;
        self.0 >= limit
    }

    fn worked(&mut self) {
        self.0 = 0;
    }
}

/// How long to leave the queue alone after messages were left in it (the tenant's backlog is full,
/// the database could not be reached): each time doubles, from the pause up to six times it. A queue
/// counts how often a message was received, and a redrive policy sends it away after a few; reading
/// at full speed while nothing can be kept would use those up.
#[derive(Debug)]
struct HoldOff {
    base: Duration,
    next: Option<Duration>,
}

impl HoldOff {
    const fn new(base: Duration) -> Self {
        Self { base, next: None }
    }

    fn grow(&mut self) -> Duration {
        let wait = self
            .next
            .map_or(self.base, |last| (last * 2).min(self.base * 6));
        self.next = Some(wait);
        wait
    }

    fn clear(&mut self) {
        self.next = None;
    }
}

/// A message as the queue gave it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Received {
    pub message_id: String,
    pub receipt_handle: String,
    pub body: String,
    /// String message attributes, by name.
    pub attributes: BTreeMap<String, String>,
}

/// The answer is not well-formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreadable;

/// The messages of a `ReceiveMessage` answer.
///
/// # Errors
/// [`Unreadable`] when the answer is not well-formed XML.
pub fn parse_receive(xml: &str) -> Result<Vec<Received>, Unreadable> {
    let mut reader = Reader::from_str(xml);
    let mut messages = Vec::new();
    let mut current: Option<Received> = None;
    // The names of the elements being read, to know what a piece of text belongs to; and the text
    // of the one being read (it arrives in pieces when it has entities in it).
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut attribute_name = String::new();
    let mut attribute_value: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                let name = start.name().as_ref().to_owned();
                if name == "Message" {
                    current = Some(Received::default());
                }
                if name == "MessageAttribute" {
                    attribute_name.clear();
                    attribute_value = None;
                }
                text.clear();
                path.push(name);
            }
            Ok(Event::Text(piece)) => text.push_str(&piece.xml10_content()),
            Ok(Event::GeneralRef(reference)) => {
                if let Some(c) = reference.resolve_char_ref().map_err(|_| Unreadable)? {
                    text.push(c);
                } else {
                    text.push_str(match &*reference {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => return Err(Unreadable),
                    });
                }
            }
            Ok(Event::End(end)) => {
                let name = end.name().as_ref().to_owned();
                let parent = path.len().checked_sub(2).and_then(|i| path.get(i)).cloned();
                if let Some(message) = current.as_mut() {
                    match (name.as_str(), parent.as_deref()) {
                        ("MessageId", Some("Message")) => message.message_id = text.clone(),
                        ("ReceiptHandle", Some("Message")) => message.receipt_handle = text.clone(),
                        ("Body", Some("Message")) => message.body = text.clone(),
                        ("Name", Some("MessageAttribute")) => attribute_name = text.clone(),
                        ("StringValue", Some("Value")) => attribute_value = Some(text.clone()),
                        _ => {}
                    }
                }
                if name == "MessageAttribute"
                    && let (Some(message), Some(value)) = (current.as_mut(), attribute_value.take())
                {
                    message
                        .attributes
                        .insert(std::mem::take(&mut attribute_name), value);
                }
                if name == "Message"
                    && let Some(message) = current.take()
                {
                    messages.push(message);
                }
                text.clear();
                path.pop();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(Unreadable),
        }
    }
    Ok(messages)
}

/// A signed Query request to the queue; the answer's body.
pub(crate) async fn call(
    client: &EgressClient,
    queue_url: &str,
    region: &str,
    credentials: &Credentials,
    parameters: &[(String, String)],
) -> Result<String, &'static str> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(parameters.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .finish()
        .into_bytes();
    let mut headers: Vec<(String, String)> = vec![(
        "content-type".to_owned(),
        "application/x-www-form-urlencoded".to_owned(),
    )];
    if let (Some(key), Some(secret)) = (
        credentials.text("accessKeyId"),
        credentials.text("secretAccessKey"),
    ) {
        let identity = AwsCredentials::new(
            key,
            secret,
            credentials.text("sessionToken").map(str::to_owned),
            None,
            "sagat",
        )
        .into();
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(region)
            .name("sqs")
            .time(SystemTime::now())
            .settings(SigningSettings::default())
            .build()
            .map_err(|_| "the SQS request cannot be signed")?
            .into();
        let signable = SignableRequest::new(
            "POST",
            queue_url,
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            SignableBody::Bytes(&body),
        )
        .map_err(|_| "the SQS request cannot be signed")?;
        let (instructions, _) = sign(signable, &params)
            .map_err(|_| "the SQS request cannot be signed")?
            .into_parts();
        for (name, value) in instructions.headers() {
            headers.push((name.to_owned(), value.to_owned()));
        }
    }
    let response = client
        .send_with(
            EgressRequest {
                method: Method::POST,
                url: queue_url.to_owned(),
                headers,
                body,
            },
            ResponseBody::Keep(MAX_RESPONSE_BYTES),
        )
        .await
        .map_err(|error| match error {
            sagat_infra_egress::EgressError::ResponseTooLarge => {
                "SQS answered with more than a batch can be"
            }
            _ => "could not reach the SQS endpoint",
        })?;
    if !response.status.is_success() {
        return Err("SQS refused the request");
    }
    String::from_utf8(response.body).map_err(|_| "SQS answered with something unreadable")
}

fn pair(name: &str, value: impl ToString) -> (String, String) {
    (name.to_owned(), value.to_string())
}

/// Reads the queue until told to stop or until something fails.
pub async fn run<R: ConnectorRuntime>(
    ingest: Arc<Ingest<R>>,
    client: Arc<EgressClient>,
    credentials: Credentials,
    stop: watch::Receiver<bool>,
) -> Stopped {
    run_with(ingest, client, credentials, stop, Timing::default()).await
}

/// As [`run`], with its waits and its patience given.
pub async fn run_with<R: ConnectorRuntime>(
    ingest: Arc<Ingest<R>>,
    client: Arc<EgressClient>,
    credentials: Credentials,
    mut stop: watch::Receiver<bool>,
    timing: Timing,
) -> Stopped {
    let connector = ingest.connector();
    let ConnectorSettings::Sqs { queue_url, region } = &connector.settings else {
        return Stopped::Failed("the connector is not an SQS connector");
    };
    let mut pacer = Pacer::new(connector.rate_per_second);
    let receive = [
        pair("Action", "ReceiveMessage"),
        pair("Version", "2012-11-05"),
        pair("QueueUrl", queue_url),
        pair("MaxNumberOfMessages", BATCH),
        pair("WaitTimeSeconds", WAIT_SECONDS),
        pair("VisibilityTimeout", VISIBILITY_SECONDS),
        pair("MessageAttributeName.1", "All"),
    ];
    let mut streak = Streak::default();
    let mut hold_off = HoldOff::new(timing.pause);
    loop {
        let received = tokio::select! {
            () = crate::supervisor::stopped(&mut stop) => {
                ingest.flush(true).await;
                return Stopped::Asked;
            }
            received = call(&client, queue_url, region, &credentials, &receive) => received,
        };
        let messages = match received.and_then(|xml| {
            parse_receive(&xml).map_err(|Unreadable| "SQS answered with something unreadable")
        }) {
            Ok(messages) => messages,
            Err(why) => {
                ingest.failed(why).await;
                if streak.failed(timing.restart_after) {
                    // Started again, the reader reads its credentials and its policy anew.
                    return Stopped::Failed(why);
                }
                tokio::select! {
                    () = crate::supervisor::stopped(&mut stop) => return Stopped::Asked,
                    () = tokio::time::sleep(timing.pause) => {}
                }
                continue;
            }
        };
        streak.worked();
        let mut delete = vec![
            pair("Action", "DeleteMessageBatch"),
            pair("Version", "2012-11-05"),
            pair("QueueUrl", queue_url),
        ];
        let mut deletable = 0;
        let mut left = false;
        for (index, message) in messages.iter().enumerate() {
            pacer.wait().await;
            let raw = RawMessage {
                position: message.message_id.clone(),
                fallback_id: message.message_id.clone(),
                headers: message.attributes.clone(),
                body: message.body.as_bytes().to_vec(),
            };
            match ingest.ingest(&raw, None).await {
                Outcome::Admitted | Outcome::Duplicate | Outcome::SetAside(_) => {
                    deletable += 1;
                    delete.push(pair(
                        &format!("DeleteMessageBatchRequestEntry.{deletable}.Id"),
                        index,
                    ));
                    delete.push(pair(
                        &format!("DeleteMessageBatchRequestEntry.{deletable}.ReceiptHandle"),
                        &message.receipt_handle,
                    ));
                }
                // Left in the queue: it becomes visible again when its timeout ends.
                Outcome::Backlogged | Outcome::Failed | Outcome::Superseded => left = true,
            }
        }
        if deletable > 0
            && let Err(why) = call(&client, queue_url, region, &credentials, &delete).await
        {
            // The messages are in the inbox; the queue delivers them again and the inbox says so.
            ingest.failed(why).await;
        }
        if messages.is_empty() {
            ingest.flush(false).await;
        }
        if left {
            let wait = hold_off.grow();
            tokio::select! {
                () = crate::supervisor::stopped(&mut stop) => return Stopped::Asked,
                () = tokio::time::sleep(wait) => {}
            }
        } else {
            hold_off.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_gives_up_only_after_the_failures_come_in_a_row() {
        let mut streak = Streak::default();
        assert!(!streak.failed(3));
        assert!(!streak.failed(3));
        streak.worked();
        assert!(
            !streak.failed(3),
            "a call that worked starts the count again"
        );
        assert!(!streak.failed(3));
        assert!(streak.failed(3));
    }

    #[test]
    fn the_wait_after_leaving_messages_grows_to_a_ceiling_and_starts_over() {
        let mut hold = HoldOff::new(Duration::from_secs(5));
        let waits: Vec<u64> = (0..5).map(|_| hold.grow().as_secs()).collect();
        assert_eq!(waits, [5, 10, 20, 30, 30]);
        hold.clear();
        assert_eq!(hold.grow(), Duration::from_secs(5));
    }

    #[test]
    fn a_receive_answer_becomes_messages_with_their_text_attributes() {
        let xml = r#"<?xml version="1.0"?>
<ReceiveMessageResponse xmlns="http://queue.amazonaws.com/doc/2012-11-05/">
  <ReceiveMessageResult>
    <Message>
      <MessageId>m-1</MessageId>
      <ReceiptHandle>handle==</ReceiptHandle>
      <MD5OfBody>abc</MD5OfBody>
      <Body>{&quot;amount&quot;:10,&quot;note&quot;:&quot;a &amp; b&quot;}</Body>
      <MessageAttribute><Name>eventType</Name><Value><DataType>String</DataType><StringValue>order.created</StringValue></Value></MessageAttribute>
      <MessageAttribute><Name>blob</Name><Value><DataType>Binary</DataType><BinaryValue>AAEC</BinaryValue></Value></MessageAttribute>
    </Message>
    <Message>
      <MessageId>m-2</MessageId>
      <ReceiptHandle>h2</ReceiptHandle>
      <Body>{}</Body>
    </Message>
  </ReceiveMessageResult>
  <ResponseMetadata><RequestId>r</RequestId></ResponseMetadata>
</ReceiveMessageResponse>"#;
        let messages = parse_receive(xml).expect("parsed");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message_id, "m-1");
        assert_eq!(messages[0].receipt_handle, "handle==");
        assert_eq!(messages[0].body, r#"{"amount":10,"note":"a & b"}"#);
        assert_eq!(messages[0].attributes["eventType"], "order.created");
        assert!(
            !messages[0].attributes.contains_key("blob"),
            "a binary attribute is not text"
        );
        assert_eq!(messages[1].body, "{}");
        assert!(messages[1].attributes.is_empty());
    }

    #[test]
    fn an_empty_queue_is_no_messages_and_garbage_is_an_error() {
        let empty = r#"<ReceiveMessageResponse><ReceiveMessageResult/></ReceiveMessageResponse>"#;
        assert_eq!(parse_receive(empty), Ok(vec![]));
        assert_eq!(parse_receive("<a><b></a>"), Err(Unreadable));
    }
}
