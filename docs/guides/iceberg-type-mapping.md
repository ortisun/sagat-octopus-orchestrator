# How a record becomes a row of an Iceberg table

An `icebergWrite` step stores a value as it is, or refuses the write. Nothing is rounded, truncated, wrapped, coerced, given a
zone it did not have or stripped of the one it had. This page is the published mapping table (FR-035-010, D035-091). The code
that applies it is `runners/lake/sagat_lake/mapping.py`, and its tests are `runners/lake/tests/test_mapping.py`.

A refusal is `ICEBERG_SCHEMA_MISMATCH` with the **column** (the path of a nested one, `customer.address.zip`) and a
**reason**. It never carries the value.

| Iceberg type | What the record may hold | Refused as |
|---|---|---|
| `boolean` | `true` or `false` | `TYPE` |
| `int`, `long` | a JSON integer, or a string of ASCII digits (with an optional `-`) for a value a JSON number cannot hold in every language | a fraction, text, a boolean, a sign `+`, a space or a digit of another script: `TYPE`; outside the width: `OVERFLOW` |
| `float`, `double` | a finite JSON number; also `NaN` and the infinities, which only these columns can hold | text or a boolean: `TYPE`; a finite value past the largest `float`, or one that is not zero and so small that the column would store zero: `OVERFLOW` |
| `decimal(p, s)` | a string of digits with an optional point, or an exact JSON number | a digit that would be rounded away: `SCALE_OVERFLOW` (trailing zeros carry nothing and are accepted); more digits than `p`: `OVERFLOW`; a float, `NaN`, text: `TYPE` |
| `date` | `YYYY-MM-DD` | `TYPE` |
| `time` | `HH:MM:SS` with up to six fractional digits, **no zone** | a zone: `ZONE`; a digit past the sixth that is not zero: `SCALE_OVERFLOW` |
| `timestamp` | `YYYY-MM-DDTHH:MM:SS[.f]`, **no zone** | a zone (it would be dropped): `ZONE`; sub-microsecond digits: `SCALE_OVERFLOW` |
| `timestamptz` | the same with `Z` or an offset; stored as UTC | no zone (it would be guessed): `ZONE` |
| `string` | a JSON string | `TYPE` |
| `uuid` | the canonical form of 36 characters | `TYPE` |
| `binary`, `fixed(n)` | the typed cell `{"type": "binary", "value": "<padded base64>"}` of feature 017 | a bare string, base64 that is not padded or has characters outside the alphabet, or a `fixed` value of another length: `TYPE` |
| `struct` | a JSON object, field by field | a key the table does not have: `UNKNOWN_FIELD`, always |
| `list<T>` | a JSON array, element by element | `TYPE` |
| `map<K, V>` | an array of `{"key": k, "value": v}`, or an object when the keys are strings | the same key twice, or an object for keys that are not strings: `TYPE` |

Around the types:

- A **missing optional column** becomes null. A missing required column is `MISSING_REQUIRED`, and a null in a required column
  is `NULL_IN_REQUIRED` (inside a list, a map or a struct too).
- A **field of the record with no column** is `UNKNOWN_FIELD`, unless the step says `ignoreUnknownFields`: such fields are then
  dropped before any check, never leave the platform, and are counted in the output. A nested field the table does not have
  is always refused.
- The **mapping** of the step reads a field by JSON pointer (`/customer/name`, with `~1` for `/` and `~0` for `~`, and a
  number for a position of an array) and writes a column. A field the mapping does not name is not written, and is not an
  unknown field. A mapping to a column the table does not have is `UNKNOWN_FIELD`.
- Numbers are read with their digits (`json.loads(line, parse_float=Decimal)`), so `12.50` into a `decimal(12, 2)` stays
  `12.50`.

The step never adds, drops or alters a column of the table.

## When the records are the result of a SQL step

A write that names the node of a SQL step (`records: {"databaseResult": "<node>"}`, only a `query` step) takes the rows that step
retained. The platform turns each row into a record, one object keyed by the column, and each cell is the value the SQL step kept, so
the table above applies to it as it does to any record. What follows is what each cell does for each type of column; it is the
list the lease and the host are both tested against (`tests/contract/iceberg-writes/fixtures/result-records/cells.json`).

| The cell | Becomes | A column of | Refused as |
|---|---|---|---|
| `integer` | the string of its digits | `int`, `long`, `decimal`, `string` | `double` and `float`: `TYPE`; `int`: `OVERFLOW` past 32 bits |
| `decimal` | the string of its digits | `decimal(p, s)` | `long`, `double`, `float`: `TYPE`; a digit the column would round away: `SCALE_OVERFLOW` |
| `float` | a JSON number | `double`, `float` | `long`, `string`: `TYPE` (a SQL `NaN` or an infinity is not kept by the SQL step, so none arrives) |
| `text` | the string | `string`, and `uuid` for a canonical UUID | everything else: `TYPE` |
| `boolean` | `true` or `false` | `boolean` | `TYPE` |
| `timestamp` | text with its offset, always: the SQL step keeps only `timestamptz` | `timestamptz`, `string` | `timestamp` (no zone): `ZONE`; `date`: `TYPE` |
| `date` | `YYYY-MM-DD` | `date`, `string` | `timestamptz`: `TYPE` |
| `binary` | the typed cell `{"type": "binary", "value": "<base64>"}` | `binary`, `fixed(n)` of that length | `string`: `TYPE` |
| `json` | the JSON value itself | a `struct`, `list` or `map` column reads its structure; a JSON string goes into `string` | an object or a list into `string`: `TYPE` (the host never turns structure into text) |
| a SQL `NULL` | `null` | any optional column | a required column: `NULL_IN_REQUIRED` (a JSON `null` in a `json` cell is the same `null`) |

Three things follow that the SQL step cannot be asked to do for you. A SQL integer does not go into a `double` column, and a
`timestamp without time zone` is not kept by the SQL step at all, so a result cannot fill a column of type `timestamp`: both are
refused loudly (`TYPE`, `ZONE`), never rounded or guessed, and the step has no transform that could fix them (G035-082). A `json`
cell whose value is exactly an object with the two members `type` (`"binary"`) and `value` is refused by the lease, because the host
would read it as bytes. A statement that selects no column, two columns of one name or a column of no name cannot be records and fail
the write for good (`DATABASE_CONFIGURATION_INVALID`); a result above 256 KiB is kept as a blob and is refused by name (G035-067).
