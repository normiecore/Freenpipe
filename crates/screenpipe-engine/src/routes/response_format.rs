// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Shared response shaping for list endpoints: `format=` and `fields=`.
//!
//! `/search`, `/elements` and `/frames/:id/elements` take two optional query
//! parameters:
//!
//! - `format`: `json` (the default), `csv` or `tsv`. Anything else is a 400
//!   with `unknown format `<value>` (expected json|csv|tsv)`.
//! - `fields`: a comma-separated list of dotted paths into each row, such as
//!   `type,content.app_name,content.text` for `/search` or
//!   `id,role,bounds.left` for `/elements`. A numeric segment indexes an
//!   array (`content.tags.0`).
//!
//! With neither parameter the handler's JSON goes out untouched, so existing
//! clients see exactly the shape they always did.
//!
//! The rows are the elements of the response's `data` array (or the response
//! itself when it is a bare array). What happens to them:
//!
//! - **JSON with `fields`**: each row becomes a flat object keyed by the
//!   requested paths, in the requested order (`{"type": ..,
//!   "content.text": ..}`). A path a row does not have is left out of that
//!   row. Everything outside `data` (`pagination` and so on) is kept.
//! - **CSV / TSV**: one header row, then one line per row. The columns are
//!   the `fields` paths, or without `fields` every leaf path seen in any row,
//!   in first-seen order. Nested objects flatten into dotted column names; an
//!   array is one cell holding its compact JSON; null and missing values are
//!   empty cells. CSV follows RFC 4180 (CRLF line ends; a cell holding a
//!   comma, quote, CR or LF is quoted and its quotes doubled), so it is
//!   lossless. TSV has no quoting, so tabs and line breaks inside a cell
//!   become single spaces. Tabular output has no envelope; when the JSON
//!   carried `pagination.total` it is sent as the `x-total-count` header.
//!
//! Tabular output is built from the same rows the JSON would carry, so it is
//! bounded by the same `limit` and paging as the JSON response.
//!
//! This module serves no routes of its own; it is listed in
//! [`super::ext`] like the other feature modules.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use axum::{
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{
    de::{MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
    Deserialize, Deserializer, Serialize, Serializer,
};
use serde_json::{json, Value};
use tracing::error;

use super::ext::{DocumentedRoutes, ExtRouter};

/// Content type of `format=csv` responses.
pub const CSV_CONTENT_TYPE: &str = "text/csv; charset=utf-8";
/// Content type of `format=tsv` responses.
pub const TSV_CONTENT_TYPE: &str = "text/tab-separated-values; charset=utf-8";
/// Header carrying `pagination.total` on tabular responses.
pub const TOTAL_COUNT_HEADER: &str = "x-total-count";

/// Routes described in `/openapi.json`: none, this module is a library.
pub fn documented() -> DocumentedRoutes {
    DocumentedRoutes::empty()
}

/// Routes outside the OpenAPI spec: none.
pub fn router() -> ExtRouter {
    ExtRouter::new()
}

/// The value of `format=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseFormat {
    #[default]
    Json,
    Csv,
    Tsv,
}

impl ResponseFormat {
    /// Parse a `format=` value. Absent or empty means JSON; names are matched
    /// without regard to case or surrounding space.
    pub fn parse(raw: Option<&str>) -> Result<Self, UnknownFormat> {
        let Some(raw) = raw else {
            return Ok(Self::Json);
        };
        let name = raw.trim();
        if name.is_empty() || name.eq_ignore_ascii_case("json") {
            Ok(Self::Json)
        } else if name.eq_ignore_ascii_case("csv") {
            Ok(Self::Csv)
        } else if name.eq_ignore_ascii_case("tsv") {
            Ok(Self::Tsv)
        } else {
            Err(UnknownFormat(raw.to_string()))
        }
    }

    /// The `Content-Type` this format is sent with.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Csv => CSV_CONTENT_TYPE,
            Self::Tsv => TSV_CONTENT_TYPE,
        }
    }
}

/// A `format=` value that is not one of the supported formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFormat(pub String);

impl fmt::Display for UnknownFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown format `{}` (expected json|csv|tsv)", self.0)
    }
}

impl std::error::Error for UnknownFormat {}

impl UnknownFormat {
    /// The 400 response handlers return for it: `{"error": "<message>"}`.
    pub fn into_rejection(self) -> (StatusCode, Json<Value>) {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": self.to_string() })),
        )
    }
}

/// One dotted path from `fields=`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldPath {
    /// The path as written, used as the JSON key and the column name.
    name: String,
    segments: Vec<String>,
}

/// The parsed `fields=` list: trimmed, empty entries dropped, duplicates
/// kept once at their first position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSelection {
    paths: Vec<FieldPath>,
}

impl FieldSelection {
    /// `None` when the parameter is absent or names no path at all.
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        let mut paths: Vec<FieldPath> = Vec::new();
        for entry in raw?.split(',') {
            let name = entry.trim();
            if name.is_empty() || paths.iter().any(|p| p.name == name) {
                continue;
            }
            paths.push(FieldPath {
                name: name.to_string(),
                segments: name.split('.').map(|s| s.trim().to_string()).collect(),
            });
        }
        if paths.is_empty() {
            None
        } else {
            Some(Self { paths })
        }
    }

    /// The paths in request order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.paths.iter().map(|p| p.name.as_str())
    }
}

/// How a list response should be rendered: the parsed `format` and `fields`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseShape {
    format: ResponseFormat,
    fields: Option<FieldSelection>,
}

impl ResponseShape {
    /// Parse the two query parameters.
    pub fn parse(format: Option<&str>, fields: Option<&str>) -> Result<Self, UnknownFormat> {
        Ok(Self {
            format: ResponseFormat::parse(format)?,
            fields: FieldSelection::parse(fields),
        })
    }

    /// [`Self::parse`] with the error already turned into the handlers' 400
    /// response, so a handler can write `ResponseShape::from_query(..)?`.
    pub fn from_query(
        format: Option<&str>,
        fields: Option<&str>,
    ) -> Result<Self, (StatusCode, Json<Value>)> {
        Self::parse(format, fields).map_err(UnknownFormat::into_rejection)
    }

    pub fn format(&self) -> ResponseFormat {
        self.format
    }

    pub fn fields(&self) -> Option<&FieldSelection> {
        self.fields.as_ref()
    }

    /// True when neither parameter changes anything: plain JSON goes out.
    pub fn is_default(&self) -> bool {
        self.format == ResponseFormat::Json && self.fields.is_none()
    }

    /// Wrap a handler's JSON body so it is rendered in this shape.
    pub fn respond<T>(self, body: T) -> Shaped<T> {
        Shaped { shape: self, body }
    }

    /// Render an already serialised response body.
    fn render(&self, envelope: Node) -> Response {
        let dialect = match self.format {
            ResponseFormat::Json => {
                return Json(self.project_envelope(envelope)).into_response();
            }
            ResponseFormat::Csv => Dialect::Csv,
            ResponseFormat::Tsv => Dialect::Tsv,
        };
        let total = envelope
            .get_path(&["pagination", "total"])
            .and_then(Node::as_integer);
        let rows = envelope.into_rows();
        let mut response = render_table(&rows, self.fields.as_ref(), dialect).into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(self.format.content_type()),
        );
        if let Some(total) = total {
            headers.insert(
                HeaderName::from_static(TOTAL_COUNT_HEADER),
                HeaderValue::from(total),
            );
        }
        response
    }

    /// JSON output: project every row of `data` (or of a bare array). The
    /// rows are the same ones [`Node::into_rows`] gives tabular output, so a
    /// body without a `data` array is projected as one row.
    fn project_envelope(&self, envelope: Node) -> Node {
        let Some(fields) = self.fields.as_ref() else {
            return envelope;
        };
        match envelope {
            Node::Array(rows) => Node::Array(project_rows(&rows, fields)),
            Node::Object(entries) if has_data_rows(&entries) => {
                let mut projected = false;
                Node::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| match value {
                            Node::Array(rows) if key == "data" && !projected => {
                                projected = true;
                                (key, Node::Array(project_rows(&rows, fields)))
                            }
                            value => (key, value),
                        })
                        .collect(),
                )
            }
            single => project_row(&single, fields),
        }
    }
}

/// A handler's JSON body plus the shape to render it in. List handlers
/// return it in place of `Json<T>`; the OpenAPI spec still describes `T`,
/// the default JSON body.
pub struct Shaped<T> {
    shape: ResponseShape,
    body: T,
}

impl<T: Serialize> IntoResponse for Shaped<T> {
    fn into_response(self) -> Response {
        if self.shape.is_default() {
            return Json(self.body).into_response();
        }
        // Going through text keeps each object's field order, which
        // `serde_json::Value` would sort away.
        let envelope =
            serde_json::to_vec(&self.body).and_then(|bytes| serde_json::from_slice::<Node>(&bytes));
        match envelope {
            Ok(envelope) => self.shape.render(envelope),
            Err(e) => {
                error!("failed to format response: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("failed to format response: {}", e) })),
                )
                    .into_response()
            }
        }
    }
}

impl<T: oasgen::OaSchema> oasgen::OaParameter for Shaped<T> {
    fn body_schema() -> Option<oasgen::RefOr<oasgen::Schema>> {
        T::body_schema()
    }
}

/// A JSON value whose objects keep their key order.
#[derive(Debug, Clone, PartialEq)]
enum Node {
    /// null, a boolean, a number or a string.
    Scalar(Value),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

impl Node {
    fn get(&self, segment: &str) -> Option<&Node> {
        match self {
            Node::Object(entries) => entries.iter().find(|(k, _)| k == segment).map(|(_, v)| v),
            Node::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
            Node::Scalar(_) => None,
        }
    }

    fn get_path<S: AsRef<str>>(&self, segments: &[S]) -> Option<&Node> {
        segments
            .iter()
            .try_fold(self, |node, segment| node.get(segment.as_ref()))
    }

    fn as_integer(&self) -> Option<i64> {
        match self {
            Node::Scalar(Value::Number(n)) => n.as_i64(),
            _ => None,
        }
    }

    /// The rows of a list response: `data` of an object envelope, the items
    /// of a bare array, or the value itself as a single row.
    fn into_rows(self) -> Vec<Node> {
        match self {
            Node::Array(rows) => rows,
            Node::Object(entries) => {
                let mut rest = Vec::new();
                let mut data = None;
                for (key, value) in entries {
                    match value {
                        Node::Array(rows) if key == "data" && data.is_none() => data = Some(rows),
                        value => rest.push((key, value)),
                    }
                }
                data.unwrap_or_else(|| vec![Node::Object(rest)])
            }
            scalar => vec![scalar],
        }
    }

    /// Text of one table cell.
    fn cell_text(&self) -> Cow<'_, str> {
        match self {
            Node::Scalar(Value::Null) => Cow::Borrowed(""),
            Node::Scalar(Value::String(s)) => Cow::Borrowed(s),
            Node::Scalar(other) => Cow::Owned(other.to_string()),
            compound => Cow::Owned(serde_json::to_string(compound).unwrap_or_default()),
        }
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Node::Scalar(value) => value.serialize(serializer),
            Node::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Node::Object(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Node, E> {
        Ok(Node::Scalar(Value::Bool(v)))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Node, E> {
        Ok(Node::Scalar(Value::from(v)))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Node, E> {
        Ok(Node::Scalar(Value::from(v)))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Node, E> {
        Ok(Node::Scalar(Value::from(v)))
    }

    fn visit_str<E>(self, v: &str) -> Result<Node, E> {
        Ok(Node::Scalar(Value::String(v.to_string())))
    }

    fn visit_string<E>(self, v: String) -> Result<Node, E> {
        Ok(Node::Scalar(Value::String(v)))
    }

    fn visit_unit<E>(self) -> Result<Node, E> {
        Ok(Node::Scalar(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Node, E> {
        Ok(Node::Scalar(Value::Null))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        Node::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Node::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, value)) = map.next_entry::<String, Node>()? {
            entries.push((key, value));
        }
        Ok(Node::Object(entries))
    }
}

/// Whether an object envelope carries its rows in a `data` array.
fn has_data_rows(entries: &[(String, Node)]) -> bool {
    entries
        .iter()
        .any(|(key, value)| key == "data" && matches!(value, Node::Array(_)))
}

fn project_rows(rows: &[Node], fields: &FieldSelection) -> Vec<Node> {
    rows.iter().map(|row| project_row(row, fields)).collect()
}

/// One row reduced to the requested paths, as a flat object.
fn project_row(row: &Node, fields: &FieldSelection) -> Node {
    Node::Object(
        fields
            .paths
            .iter()
            .filter_map(|path| {
                row.get_path(&path.segments)
                    .map(|value| (path.name.clone(), value.clone()))
            })
            .collect(),
    )
}

/// Every leaf of `node` with its dotted path. Objects are walked; arrays,
/// scalars and empty objects are leaves. A row that is not an object at
/// all is one leaf named `value`.
fn flatten<'a>(node: &'a Node, prefix: &mut String, out: &mut Vec<(String, &'a Node)>) {
    match node {
        Node::Object(entries) if !entries.is_empty() => {
            for (key, value) in entries {
                let restore = prefix.len();
                if !prefix.is_empty() {
                    prefix.push('.');
                }
                prefix.push_str(key);
                flatten(value, prefix, out);
                prefix.truncate(restore);
            }
        }
        leaf => {
            let name = if prefix.is_empty() {
                "value".to_string()
            } else {
                prefix.clone()
            };
            out.push((name, leaf));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Csv,
    Tsv,
}

impl Dialect {
    fn separator(self) -> char {
        match self {
            Dialect::Csv => ',',
            Dialect::Tsv => '\t',
        }
    }

    fn line_end(self) -> &'static str {
        match self {
            Dialect::Csv => "\r\n",
            Dialect::Tsv => "\n",
        }
    }

    fn push_cell(self, out: &mut String, text: &str) {
        match self {
            Dialect::Csv => push_csv_cell(out, text),
            Dialect::Tsv => push_tsv_cell(out, text),
        }
    }

    fn push_line<'a>(self, out: &mut String, cells: impl Iterator<Item = Cow<'a, str>>) {
        for (i, cell) in cells.enumerate() {
            if i > 0 {
                out.push(self.separator());
            }
            self.push_cell(out, &cell);
        }
        out.push_str(self.line_end());
    }
}

/// RFC 4180: quote a cell that holds a comma, a quote, CR or LF, doubling
/// the quotes inside it.
fn push_csv_cell(out: &mut String, text: &str) {
    if !text.contains(|c| matches!(c, ',' | '"' | '\r' | '\n')) {
        out.push_str(text);
        return;
    }
    out.push('"');
    for ch in text.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
}

/// TSV has no quoting: a tab, a CR, an LF or a CRLF pair becomes one space.
fn push_tsv_cell(out: &mut String, text: &str) {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' | '\t' => out.push(' '),
            other => out.push(other),
        }
    }
}

/// Render rows as CSV or TSV text with one header line. With no rows and no
/// `fields` there are no columns, so the body is empty.
fn render_table(rows: &[Node], fields: Option<&FieldSelection>, dialect: Dialect) -> String {
    let mut out = String::new();
    if let Some(fields) = fields {
        dialect.push_line(&mut out, fields.names().map(Cow::Borrowed));
        for row in rows {
            let cells = fields.paths.iter().map(|path| {
                row.get_path(&path.segments)
                    .map(Node::cell_text)
                    .unwrap_or(Cow::Borrowed(""))
            });
            dialect.push_line(&mut out, cells);
        }
        return out;
    }

    let flat: Vec<Vec<(String, &Node)>> = rows
        .iter()
        .map(|row| {
            let mut leaves = Vec::new();
            flatten(row, &mut String::new(), &mut leaves);
            leaves
        })
        .collect();
    let mut columns: Vec<&str> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (name, _) in flat.iter().flatten() {
        if !index.contains_key(name.as_str()) {
            index.insert(name.as_str(), columns.len());
            columns.push(name.as_str());
        }
    }
    if columns.is_empty() {
        return out;
    }
    dialect.push_line(&mut out, columns.iter().map(|c| Cow::Borrowed(*c)));
    for leaves in &flat {
        let mut cells: Vec<Option<&Node>> = vec![None; columns.len()];
        for (name, value) in leaves {
            cells[index[name.as_str()]] = Some(*value);
        }
        dialect.push_line(
            &mut out,
            cells
                .into_iter()
                .map(|cell| cell.map(Node::cell_text).unwrap_or(Cow::Borrowed(""))),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn node(value: Value) -> Node {
        serde_json::from_value(value).unwrap()
    }

    fn shape(format: Option<&str>, fields: Option<&str>) -> ResponseShape {
        ResponseShape::parse(format, fields).unwrap()
    }

    struct Rendered {
        status: StatusCode,
        content_type: Option<String>,
        total: Option<String>,
        body: String,
    }

    async fn rendered(response: Response) -> Rendered {
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .map(|v| v.to_str().unwrap().to_string())
        };
        let content_type = header("content-type");
        let total = header(TOTAL_COUNT_HEADER);
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Rendered {
            status,
            content_type,
            total,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    /// A body parsed from JSON text, keys kept in the order written (a
    /// `json!` value would sort them).
    fn doc(text: &str) -> Node {
        serde_json::from_str(text).unwrap()
    }

    fn sample() -> Node {
        doc(concat!(
            r#"{"data":["#,
            r#"{"type":"OCR","content":{"frame_id":7,"text":"hello, \"world\"","tags":["a","b"],"focused":true}},"#,
            r#"{"type":"Audio","content":{"chunk_id":3,"text":"line one\nline two\tend","speaker":null}}"#,
            r#"],"pagination":{"limit":2,"offset":0,"total":9}}"#
        ))
    }

    #[test]
    fn format_names_parse_and_unknown_ones_are_rejected() {
        assert_eq!(ResponseFormat::parse(None), Ok(ResponseFormat::Json));
        assert_eq!(ResponseFormat::parse(Some("")), Ok(ResponseFormat::Json));
        assert_eq!(
            ResponseFormat::parse(Some("json")),
            Ok(ResponseFormat::Json)
        );
        assert_eq!(ResponseFormat::parse(Some("CSV")), Ok(ResponseFormat::Csv));
        assert_eq!(
            ResponseFormat::parse(Some(" tsv ")),
            Ok(ResponseFormat::Tsv)
        );
        for bad in ["xml", "table", "outline", "csv2"] {
            let err = ResponseFormat::parse(Some(bad)).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("unknown format `{bad}` (expected json|csv|tsv)")
            );
        }
        let (status, Json(body)) = UnknownFormat("xml".into()).into_rejection();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({"error": "unknown format `xml` (expected json|csv|tsv)"})
        );
        assert_eq!(
            ResponseFormat::Csv.content_type(),
            "text/csv; charset=utf-8"
        );
        assert_eq!(
            ResponseFormat::Tsv.content_type(),
            "text/tab-separated-values; charset=utf-8"
        );
    }

    #[test]
    fn field_lists_are_trimmed_and_deduplicated() {
        let fields =
            FieldSelection::parse(Some(" type , content.text,,type,content.tags.0 ")).unwrap();
        assert_eq!(
            fields.names().collect::<Vec<_>>(),
            vec!["type", "content.text", "content.tags.0"]
        );
        assert_eq!(FieldSelection::parse(None), None);
        assert_eq!(FieldSelection::parse(Some("")), None);
        assert_eq!(FieldSelection::parse(Some(" , ,")), None);
        assert!(shape(None, Some(",")).is_default());
        assert!(shape(Some("json"), None).is_default());
        assert!(!shape(Some("csv"), None).is_default());
        assert!(!shape(None, Some("type")).is_default());
    }

    #[tokio::test]
    async fn default_shape_sends_the_body_unchanged() {
        let direct = serde_json::to_vec(&sample()).unwrap();
        for s in [
            shape(None, None),
            shape(Some("json"), None),
            shape(Some(""), Some("")),
        ] {
            let r = rendered(s.respond(sample()).into_response()).await;
            assert_eq!(r.status, StatusCode::OK);
            assert_eq!(r.content_type.as_deref(), Some("application/json"));
            assert_eq!(r.total, None);
            assert_eq!(r.body.as_bytes(), direct.as_slice());
        }
    }

    #[tokio::test]
    async fn json_fields_flatten_rows_and_keep_the_envelope() {
        let response = shape(
            None,
            Some("content.text,type,content.missing,content.tags.1"),
        )
        .respond(sample())
        .into_response();
        let r = rendered(response).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.content_type.as_deref(), Some("application/json"));
        // Keys come out in the requested order; missing paths are omitted.
        assert_eq!(
            r.body,
            concat!(
                r#"{"data":[{"content.text":"hello, \"world\"","type":"OCR","content.tags.1":"b"},"#,
                r#"{"content.text":"line one\nline two\tend","type":"Audio"}],"#,
                r#""pagination":{"limit":2,"offset":0,"total":9}}"#
            )
        );
    }

    #[tokio::test]
    async fn json_fields_can_select_a_whole_subtree() {
        let response = shape(Some("json"), Some("content"))
            .respond(doc(r#"[{"content":{"b":1,"a":2}},{"other":1}]"#))
            .into_response();
        assert_eq!(
            rendered(response).await.body,
            r#"[{"content":{"b":1,"a":2}},{}]"#
        );
    }

    #[tokio::test]
    async fn csv_without_fields_uses_every_leaf_path() {
        let r = rendered(shape(Some("csv"), None).respond(sample()).into_response()).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.content_type.as_deref(), Some(CSV_CONTENT_TYPE));
        assert_eq!(r.total.as_deref(), Some("9"));
        assert_eq!(
            r.body,
            concat!(
                "type,content.frame_id,content.text,content.tags,content.focused,",
                "content.chunk_id,content.speaker\r\n",
                "OCR,7,\"hello, \"\"world\"\"\",\"[\"\"a\"\",\"\"b\"\"]\",true,,\r\n",
                "Audio,,\"line one\nline two\tend\",,,3,\r\n",
            )
        );
    }

    #[tokio::test]
    async fn csv_with_fields_follows_the_requested_columns() {
        let r = rendered(
            shape(Some("csv"), Some("content.text,type,nope"))
                .respond(sample())
                .into_response(),
        )
        .await;
        assert_eq!(
            r.body,
            concat!(
                "content.text,type,nope\r\n",
                "\"hello, \"\"world\"\"\",OCR,\r\n",
                "\"line one\nline two\tend\",Audio,\r\n",
            )
        );
    }

    #[tokio::test]
    async fn tsv_replaces_tabs_and_line_breaks() {
        let rows = doc(concat!(
            r#"{"data":[{"text":"a\tb","note":"x\r\ny\nz\rw"},"#,
            r#"{"text":"plain, \"quoted\"","note":null}]}"#
        ));
        let r = rendered(shape(Some("tsv"), None).respond(rows).into_response()).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.content_type.as_deref(), Some(TSV_CONTENT_TYPE));
        assert_eq!(r.total, None, "no pagination, no count header");
        assert_eq!(r.body, "text\tnote\na b\tx y z w\nplain, \"quoted\"\t\n");
    }

    #[tokio::test]
    async fn empty_results_keep_the_header_row_when_fields_are_given() {
        let empty = json!({"data": [], "pagination": {"limit": 5, "offset": 0, "total": 0}});
        let r = rendered(
            shape(Some("csv"), Some("id,text"))
                .respond(empty.clone())
                .into_response(),
        )
        .await;
        assert_eq!(r.body, "id,text\r\n");
        assert_eq!(r.total.as_deref(), Some("0"));
        let r = rendered(shape(Some("tsv"), None).respond(empty).into_response()).await;
        assert_eq!(r.body, "");
    }

    #[tokio::test]
    async fn nested_objects_flatten_and_arrays_stay_one_cell() {
        let rows = doc(
            r#"{"data":[{"id":1,"bounds":{"left":0.5,"top":0.25},"items":[{"k":1}],"empty":{},"note":""}]}"#,
        );
        let r = rendered(shape(Some("csv"), None).respond(rows).into_response()).await;
        assert_eq!(
            r.body,
            "id,bounds.left,bounds.top,items,empty,note\r\n1,0.5,0.25,\"[{\"\"k\"\":1}]\",{},\r\n"
        );
    }

    #[test]
    fn csv_quoting_round_trips_through_a_reader() {
        // A minimal RFC 4180 reader, to show the writer loses nothing.
        fn read(text: &str) -> Vec<Vec<String>> {
            let mut rows = vec![];
            let mut row = vec![];
            let mut cell = String::new();
            let mut chars = text.chars().peekable();
            let mut quoted = false;
            while let Some(c) = chars.next() {
                match (quoted, c) {
                    (true, '"') if chars.peek() == Some(&'"') => {
                        chars.next();
                        cell.push('"');
                    }
                    (true, '"') => quoted = false,
                    (true, c) => cell.push(c),
                    (false, '"') => quoted = true,
                    (false, ',') => row.push(std::mem::take(&mut cell)),
                    (false, '\r') if chars.peek() == Some(&'\n') => {
                        chars.next();
                        row.push(std::mem::take(&mut cell));
                        rows.push(std::mem::take(&mut row));
                    }
                    (false, c) => cell.push(c),
                }
            }
            rows
        }
        let nasty = [
            "",
            "plain",
            "a,b",
            "say \"hi\"",
            "\"",
            "line\r\nbreak",
            "lone\rcr",
            "lone\nlf",
            "  spaced  ",
            "tab\there",
            "日本語, ok",
        ];
        let rows: Vec<Node> = nasty.iter().map(|s| node(json!({ "v": s }))).collect();
        let text = render_table(&rows, None, Dialect::Csv);
        let parsed = read(&text);
        assert_eq!(parsed[0], vec!["v".to_string()]);
        let cells: Vec<String> = parsed[1..].iter().map(|r| r[0].clone()).collect();
        assert_eq!(
            cells,
            nasty.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn paths_index_arrays_and_ignore_scalars() {
        let row = node(json!({"a": {"list": [10, {"deep": "x"}]}, "s": "str"}));
        let get = |p: &str| {
            row.get_path(&p.split('.').collect::<Vec<_>>())
                .map(|n| n.cell_text().into_owned())
        };
        assert_eq!(get("a.list.0").as_deref(), Some("10"));
        assert_eq!(get("a.list.1.deep").as_deref(), Some("x"));
        assert_eq!(get("a.list.2"), None);
        assert_eq!(get("a.list.x"), None);
        assert_eq!(get("s.len"), None);
        assert_eq!(get("a.list").as_deref(), Some(r#"[10,{"deep":"x"}]"#));
    }

    #[test]
    fn rows_come_from_data_or_a_bare_array() {
        assert_eq!(node(json!([1, 2])).into_rows().len(), 2);
        assert_eq!(
            node(json!({"data": [1, 2, 3], "x": 1})).into_rows().len(),
            3
        );
        // No `data` array: the object itself is the one row.
        let rows = node(json!({"id": 1})).into_rows();
        assert_eq!(rows, vec![node(json!({"id": 1}))]);
        let text = render_table(&[node(json!(5))], None, Dialect::Csv);
        assert_eq!(text, "value\r\n5\r\n");
        // JSON projection uses the same rows.
        let s = shape(None, Some("id"));
        let single = s.project_envelope(doc(r#"{"id":1,"x":2}"#));
        assert_eq!(serde_json::to_string(&single).unwrap(), r#"{"id":1}"#);
        let enveloped = s.project_envelope(doc(r#"{"data":[{"id":1,"x":2}],"x":3}"#));
        assert_eq!(
            serde_json::to_string(&enveloped).unwrap(),
            r#"{"data":[{"id":1}],"x":3}"#
        );
    }

    #[test]
    fn node_round_trips_numbers_and_key_order() {
        let text = r#"{"z":1,"a":-2,"m":1.5,"big":18446744073709551615,"n":null,"t":true,"s":"x"}"#;
        let parsed: Node = serde_json::from_str(text).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), text);
    }
}
