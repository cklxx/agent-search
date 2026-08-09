# SOTA Framework for Agent-Friendly API Design

A synthesis of 6 research dimensions (Google/Azure/Stripe API guidelines, Anthropic & OpenAI tool-use docs, MCP specification, streaming/SSE patterns, and OpenTelemetry GenAI conventions) into a unified framework for building services that AI agents can call reliably.

---

## 1. Schema & Input Contract

The schema is the agent's primary interface. Constraints shift error prevention left — into the definition — where it costs nothing at runtime.

| Principle | Guidance |
|---|---|
| **Strict schema conformance** | Set `strict: true`, `additionalProperties: false` on every object, list all properties in `required`. Represent optionality as `"type": ["string", "null"]`, not by omitting the field. |
| **Strongest schema the input allows** | Prefer `integer` over `number`; use `enum` for closed sets; `format` for dates/URIs/emails; `minimum`/`maximum`/`minLength`/`maxLength` for bounds. Default to JSON Schema 2020-12. |
| **Make invalid states unrepresentable** | Avoid contradictory parameters (`toggle_light(on, off)`). Use typed unions over free-form strings. Group related parameters into nested objects. |
| **Parameterless tools declare empty objects** | Use `{ "type": "object", "additionalProperties": false }` — never just `{ "type": "object" }`, which accepts stray properties. |
| **Don't make the model fill arguments you already know** | Omit from the schema any value available from context (auth, session, prior tool output). Inject it server-side. |
| **Server-side validation is the trust boundary** | Validate every argument, enforce authorization per call, rate-limit. The schema is a hint for the model, not a security control. |

---

## 2. Tool & Endpoint Design

Agents select tools by matching intent to description. The description *is* the documentation.

| Principle | Guidance |
|---|---|
| **Self-documenting descriptions (Intern Test)** | 3–4 sentences: what it does, when to use it (and when not), what each parameter means, what the output is. If a new engineer couldn't use it from the description alone, add the missing answers. |
| **Consolidate chained operations** | Group actions an agent performs together into one tool (`schedule_event` finds availability *and* books). Each intermediate round-trip consumes context and accumulates error. |
| **Capability-based, not endpoint-based** | Prefer one tool with an `action` enum over a tool per REST endpoint. Offload orchestration from the agent into the tool. |
| **Constrain tool names** | ASCII letters, digits, `_`, `-`, `.`; 1–128 chars; unique within the server. Namespace by service (`github_list_prs`). |
| **Keep the loaded tool set small (<20)** | Defer rarely-used tools behind `tool_search`. A large catalog degrades selection accuracy. |
| **Distinguish read-only from mutating** | Annotate destructive/open-world tools (MCP `annotations`). Agents need this to avoid parallel calls with ordering dependencies and to apply confirmation. |
| **Provide `input_examples`** | Schema-valid examples for complex or format-sensitive parameters (dates, IDs, nested objects). Examples anchor expected format. |

**Tension — Consolidation vs. Single Responsibility:** Anthropic/OpenAI guidance favors consolidating multi-step workflows into fewer tools to cut round-trips. MCP's design principle says "one tool = one operation" for composability. Resolution: consolidate *sequential* steps the agent always performs together; keep *orthogonal* operations separate so they can be combined freely. The test is whether the agent would realistically call them independently.

---

## 3. Error Handling & Recovery

Agents can only recover from errors that tell them how. Generic `"failed"` produces dead ends.

| Principle | Guidance |
|---|---|
| **Actionable errors with recovery guidance** | State what went wrong *and* what to try next: `"Rate limit exceeded. Retry after 60 seconds."` Include the current context (e.g., current date for date-validation errors). |
| **Structured error schema** | Uniform object: `error_code`, `field` (which parameter), `message`, `example` (a valid value). Normalize network, validation, and business-logic errors into this shape. |
| **RFC 9457 for HTTP errors** | `application/problem+json` with `type` (resolvable URI), `title`, `status`, `detail`, `instance`. Validation failures include `invalid_params: [{name, reason}]`. |
| **`isError: true` for tool-level failures** | Return a successful response with `isError: true` and a corrective message. Reserve protocol/JSON-RPC errors for unknown tools or malformed requests. |
| **Field-level detail** | Pinpoint the exact parameter that's wrong and give a correct example. Removes ambiguity about expected format. |
| **Fail fast on bad input** | Reject unknown fields with `400`; validation errors return `422` with per-field detail. No silent acceptance of malformed input. |
| **Bounded retry budget with escalation** | Cap retries at 2–3. Distinguish transient (rate limit, network) from permanent (invalid params, not found). On repeated failure: try an alternative tool, degrade, or escalate — never loop. |

---

## 4. Response Design

The context window is the scarce resource. Every token in a response competes with the agent's reasoning.

| Principle | Guidance |
|---|---|
| **High-signal, token-efficient** | Return only fields the agent needs for its next decision. Offer `response_format: concise | detailed`. |
| **Semantic identifiers over opaque IDs** | Prefer `name`, `slug`, natural-language keys over UUIDs. Opaque IDs are hallucination-prone when the agent must repeat them in a subsequent call. |
| **Cursor-based pagination with `nextLink`** | Wrap lists in `{ "value": [...], "nextLink": "..." }`. Omit `nextLink` (never `null`) on the final page. Use cursors, not offset/limit. |
| **Bounded response size** | Paginate, filter, or truncate with sensible defaults. A single over-large result can overflow the context window. |
| **`outputSchema` + `structuredContent` + text** | Define `outputSchema` for typed results. Return both `structuredContent` (JSON) and a serialized text block for backward compatibility. |

---

## 5. API Conventions (REST / HTTP)

Uniformity lets agents generalize from a few examples. Predictability is the feature.

| Principle | Guidance |
|---|---|
| **Resource-oriented with standard methods** | Model around nouns; operate with `List`, `Get`, `Create`, `Update`, `Delete` mapped to HTTP verbs. |
| **Consistency across the entire surface** | kebab-case URLs, camelCase JSON, plural collection names. Identical error shape, pagination, and filtering on every endpoint. |
| **Idempotency for safe retries** | Idempotent methods (`PUT`, `DELETE`) or `Idempotency-Key` header tracked ≥5 minutes. Agents retry on failure — duplicates are the failure mode. |
| **Explicit HTTP method semantics** | `GET` safe+idempotent; `PUT`/`DELETE` idempotent; `POST`/`PATCH` neither. `DELETE` returns `204` even if the resource doesn't exist. |
| **Non-breaking versioning** | Date-stamped `api-version` query param or header (e.g., `2024-06-20`). Never in the URL path. Support old versions indefinitely. |
| **Capability discovery** | Expose `tools/list` (MCP) or OpenAPI so clients enumerate operations, schemas, and capabilities at runtime. No hardcoding. |
| **Deterministic tool ordering** | Return tools in stable order across `tools/list`. Use `listChanged` notifications for updates. Stable ordering improves prompt-cache hit rates. |

---

## 6. Streaming & Long-Running Operations

Agent runs can take minutes and connections drop. The stream must be resumable, cancellable, and unambiguous.

| Principle | Guidance |
|---|---|
| **Typed event envelope** | Every SSE frame has an `event:` field naming a lifecycle type (`message_start`, `content_block_delta`, `message_stop`, `error`). Route by type, not payload shape. |
| **Explicit terminal sentinel** | End every successful stream with a dedicated event (`message_stop`, `data: [DONE]`). Never rely on connection close alone — a dropped TCP connection looks identical to completion. |
| **In-band structured error events** | Mid-stream failures emit a typed `error` event with machine-readable `code` + `message`, then close. HTTP status can't change after `200 OK` is sent. |
| **Event IDs + `Last-Event-ID`** | Monotonic `id:` per event. On reconnect, the server replays from that offset. Avoids restarting long runs from scratch. |
| **Comment-line heartbeats** | `: ping` every 15–30s. Keeps proxies (nginx, ALB, Cloudflare) from killing idle connections during long tool calls. |
| **Disable proxy buffering** | `X-Accel-Buffering: no` (and `proxy_buffering off` in nginx). Otherwise token-by-token output gets batched and arrives late. |
| **202 Accepted + Task ID + SSE (hybrid)** | Initiating `POST` returns `202` with `Location: /events/{taskId}`. Decouples task lifecycle from a single HTTP connection; supports reconnect and polling fallback. |
| **Operation resource model** | `{ done, error, response, metadata }` (AIP-151). Works for both SSE and polling. Rule of thumb: anything >10s should be an Operation, not a blocking call. |
| **Backpressure with bounded buffers** | Bounded buffer between agent and SSE writer. Explicit drop strategy (oldest/newest/block) when full. Prevents OOM from slow consumers. |
| **Idempotency keys on stream initiation** | Deduplicate the initiating `POST` by `Idempotency-Key`. A retry must not spawn a second agent run. |
| **Cancellation on client disconnect** | When the SSE connection closes, cancel the agent context and propagate to in-flight tool calls. Don't waste compute after the client is gone. |
| **Incremental deltas with block index** | Stream partial content with an `index` identifying the content block. For tool calls, stream `input_json_delta` fragments the client accumulates. |

---

## 7. Observability

Agents fail silently (wrong answers, not errors). You need end-to-end correlation to find which step diverged.

| Principle | Guidance |
|---|---|
| **Single `trace_id` per agent run** | Every LLM call, tool invocation, retrieval, and step shares one trace, linked via `parent_span_id`. |
| **One span per operation, GenAI-named** | `gen_ai.operation.name` (`chat`, `execute_tool`, `plan`, `retrieval`, `invoke_agent`, `search_memory`) + `gen_ai.provider.name`. Standard names enable cross-provider querying. |
| **Structured (JSON) logging** | Consistent fields: `agent_id`, `session_id`, `tool_name`, `model`, `trace_id`, `level`, `timestamp`. No free-text grepping. |
| **Token usage & latency as span attributes** | `gen_ai.usage.input_tokens`, `output_tokens`, `cache_read.input_tokens`, `reasoning.output_tokens`, `duration`, `time_to_first_chunk`. Isolates the expensive step. |
| **Prompt template + model version on every call** | `gen_ai.prompt.name`, `gen_ai.prompt.version`, `gen_ai.request.model`, sampling params. Attribute regressions to the exact change. |
| **Dedicated span per tool call** | Capture tool name, arguments, result, duration — parented under the LLM span that triggered it. |
| **Conversation ID across turns** | `gen_ai.conversation.id` through every turn of a multi-turn run. Find when behavior diverged over long contexts. |
| **Opt-in content capture with PII redaction** | Prompts/responses in opt-in attributes (`gen_ai.input.messages`). Run redaction before export. |
| **Tail-based sampling** | Decide to keep a trace *after* it completes — retain errors, high-latency, specific operations. Random head sampling drops exactly the traces you need. |
| **SLO + anomaly detection over static thresholds** | Agents fail non-deterministically; fixed error-rate alerts miss quality regressions. Use error budgets and ML anomaly detection. |
| **Instrument the full stack** | Vector DB, framework steps, HTTP/DB, and LLM provider calls in one trace. Bottlenecks often live outside the model. |
| **Evaluation events on the operation span** | `gen_ai.evaluation.result` (`score.value`, `score.label`, `explanation`) parented to the LLM span. Closes the observe→experiment loop. |

---

## 8. State, Security & Trust Boundaries

| Principle | Guidance |
|---|---|
| **Explicit opaque handles, not implicit sessions** | MCP is stateless. For stateful flows (carts, transactions, browser contexts), return a high-entropy handle (UUIDv4) from a creation tool and accept it on subsequent calls. |
| **Bind handles to the authenticated user** | Never treat possession of a handle as authentication. Bind server-side to the verified user; give handles a bounded lifetime stated in the description. |
| **Server-side authorization per call** | Validate authz on every invocation. The schema and client are not trust boundaries. |

---

## 9. Discovery & Composability

| Principle | Guidance |
|---|---|
| **Runtime capability discovery** | `tools/list` + `server/discover` (MCP) or OpenAPI. Agents adapt to new tools without client redeploys. |
| **Namespaced tool names** | `{service}_{action}` keeps selection unambiguous as the catalog grows. |
| **Deferred tool loading** | Keep <20 tools upfront; defer the rest behind search. Active decision space stays small. |

---

## Top 5 Differentiating Principles (SOTA vs. Mediocre)

These five separate services agents use reliably from services agents struggle with. They are ordered by impact.

1. **Self-documenting descriptions that pass the Intern Test.** The description is the agent's only signal for *whether* and *how* to call a tool. Vague descriptions cause wrong-tool selection and malformed arguments — the dominant failure mode. Everything else (schema, errors) is downstream of getting the right tool with the right arguments.

2. **Strict schema conformance (`strict: true`, `additionalProperties: false`, all `required`).** Eliminates an entire class of runtime errors — wrong types, missing fields, extra keys — at zero runtime cost. Without it, your handler spends its budget on defensive validation and retries.

3. **Actionable, structured errors with recovery guidance.** An agent can only self-correct if the error tells it what to fix. Generic `"failed"` produces repeated failed calls or dead ends. Field-level detail + a correct example lets the agent retry correctly on the first attempt.

4. **High-signal, token-efficient responses with semantic identifiers.** The context window is the scarce resource. Bloated responses force the agent to parse irrelevant data token-by-token; opaque IDs are hallucinated when repeated in subsequent calls. This directly determines how many steps an agent can complete before running out of context.

5. **Consolidation of multi-step workflows into capability-based tools.** Each round-trip between model and tool is a source of latency and error accumulation. Collapsing sequential steps into one tool removes intermediate failure points and cuts token cost. The alternative — exposing every REST endpoint as a tool — forces the agent to be an orchestrator it isn't good at being.

---

## Tensions & Tradeoffs

| Tension | Resolution |
|---|---|
| **Consolidation vs. single responsibility** | Consolidate steps the agent *always* performs together; keep orthogonal operations separate. Test: would the agent realistically call them independently? |
| **Strict schema (all `required`) vs. optional fields** | Mark optional as `"type": ["T", "null"]` rather than omitting. The model still emits the key; you handle `null`. |
| **High-signal responses vs. completeness** | Default to `concise`; offer `detailed` via `response_format`. Let the agent choose verbosity based on its next step. |
| **Small tool surface (<20) vs. capability coverage** | Defer rarely-used tools behind `tool_search`. Keep the *active* decision space small while preserving full access. |
| **Idempotency vs. stateful operations** | Use idempotency keys for mutations; model stateful flows as explicit handles with bounded lifetimes. |
| **Streaming fidelity vs. simplicity** | For >10s operations, use the 202+Operation+SSE hybrid. For short requests, plain synchronous responses are fine. Don't stream everything. |
| **Full content capture vs. PII compliance** | Opt-in content attributes + redaction processor. Capture by default only metadata (tokens, latency, model). |
| **Tail-based sampling vs. cost** | Keep 100% of error/high-latency traces; sample the rest. The cost of missing a rare failure exceeds the storage cost. |

---

## Must-Have Checklist

### Schema & Input
- [ ] `strict: true` on every tool definition
- [ ] `additionalProperties: false` on every object
- [ ] All properties in `required`; optionality via `"type": ["T", "null"]`
- [ ] `enum` for closed sets; `integer` over `number`; `format` for dates/URIs
- [ ] No parameters for values the server already knows (auth, session)
- [ ] Server-side validation + authorization on every call

### Tool Design
- [ ] Description passes the Intern Test (3–4 sentences, covers purpose, boundaries, params, output)
- [ ] `input_examples` for complex/format-sensitive parameters
- [ ] Names conform to `[A-Za-z0-9_.-]{1,128}`, namespaced by service
- [ ] <20 tools loaded upfront; rest deferred behind search
- [ ] Read-only vs. mutating tools clearly distinguished/annotated
- [ ] Multi-step workflows consolidated into capability-based tools

### Errors
- [ ] Errors state what went wrong **and** what to try next
- [ ] Structured error object: `error_code`, `field`, `message`, `example`
- [ ] HTTP errors use RFC 9457 (`application/problem+json`)
- [ ] Tool failures return `isError: true` (not protocol errors)
- [ ] Validation failures return `422` with `invalid_params`
- [ ] Retry budget capped at 2–3 with fallback/escalation

### Responses
- [ ] Only decision-relevant fields returned by default
- [ ] `response_format: concise | detailed` offered
- [ ] Semantic identifiers (slugs/names) over opaque UUIDs
- [ ] Cursor pagination with `nextLink` (omitted, not `null`, on last page)
- [ ] Bounded response size (paginate/truncate with defaults)

### HTTP Conventions
- [ ] Resource-oriented design with standard methods
- [ ] Consistent naming (kebab-case URLs, camelCase JSON, plural collections)
- [ ] Idempotency keys on all mutating operations (≥5 min TTL)
- [ ] HTTP method safety/idempotency honored
- [ ] Date-based versioning (query param or header, never URL path)
- [ ] `tools/list` or OpenAPI discovery endpoint
- [ ] Stable tool ordering across `tools/list`

### Streaming (for >10s operations)
- [ ] Typed `event:` envelope on every SSE frame
- [ ] Explicit terminal sentinel event (not connection close)
- [ ] In-band `error` events with structured `code` + `message`
- [ ] Monotonic `id:` + `Last-Event-ID` resumability
- [ ] `: ping` heartbeats every 15–30s
- [ ] `X-Accel-Buffering: no` set
- [ ] 202 + Task ID + Operation resource (`done`/`error`/`response`/`metadata`)
- [ ] Bounded buffer with explicit drop strategy
- [ ] Idempotency key on stream initiation
- [ ] Cancellation propagated on client disconnect

### Observability
- [ ] One `trace_id` per agent run, correlated across all steps
- [ ] One span per operation with `gen_ai.operation.name`
- [ ] Structured JSON logs (no free text)
- [ ] Token usage + latency on every LLM span
- [ ] Prompt template version + model version tagged
- [ ] Dedicated `execute_tool` span with inputs/outputs
- [ ] `gen_ai.conversation.id` across all turns
- [ ] Opt-in content capture with PII redaction
- [ ] Tail-based sampling (retain errors, high-latency)
- [ ] Full-stack instrumentation (vector DB, framework, HTTP, LLM)
- [ ] Evaluation events parented to the LLM span

### State & Security
- [ ] Stateful flows use explicit opaque handles (UUIDv4)
- [ ] Handles bound to authenticated user, with bounded lifetime
- [ ] Authorization enforced per call (handle possession ≠ auth)
