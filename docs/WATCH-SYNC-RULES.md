# Watch sync — rules (pseudo-code)

The single source of behaviour. `docs/WATCH-IMPORT.md` is the reasoning; this is what
the code must do. Every `TABLE` below is written to become a unit test, row for row.

---

## Types

```rust
local    ∈ { Unset, In, Out }        // per (list_kind, title)
seen     ∈ { Absent, In }            // what the last applied tick observed remotely
remote   ∈ { Absent, In }            // what this tick observes
action   ∈ { None, AddLocal, RemoveLocal, PushAdd, PushRemove, Conflict }
```

`seen` is `cursor.listIds` (per list kind). It is advanced **only** after a tick
fully applies. "Absent" never means removed by itself.

Two preconditions, both learned the hard way (2026-11-27: a watchlist emptied
itself):

- **`remote` must come from a whole listing.** A provider asked for "what changed
  since" answers with a delta, and a delta's silence about a title says nothing
  about the list. `Pulled::lists` says which kind of answer this is
  (`ListAnswer::Whole` / `Delta`, delta by default); a delta may `AddLocal` but
  may never `RemoveLocal`, and it widens `seen` with what the last tick saw
  instead of narrowing it to its own rows. Simkl asks for the whole listing
  whenever a list's own activity stamp moved (`plantowatch`,
  `removed_from_list`, `favorites`, `custom_lists`), which is the only moment a
  removal can be there to find — and it reports `Whole` only when every bucket
  answered: a listing it refused to send whole is a delta.
- **`local` is the person's intent**, not what a pull wrote:
  `PersonalLibrary::local_membership` — the newest decision when it is theirs
  (`origin: Local`, written by the press), else the list entry. A row an import
  derived from a partial answer cannot make a kept title look removed, and the
  next whole answer brings it back.

A press writes both: the list, and a `ListDecision { state, changedAt, origin:
Local }` under no provider (`decide` with `provider = ""`). The app then asks for
a pull at once (`import::tick_now`), which is the send side: the same
reconciliation derives `PushAdd` / `PushRemove` and `trackers::deliver_list`
hands it to each enabled tracker. A tracker that cannot answer now stays owed —
the ledger still says so — and the next tick derives it again. One provider never
has two forced pulls at once; a press during one gets the second pass, and the
pass count is bounded (`FORCED_PASSES`).

TABLE 2 is **not implemented**: a membership carries no remote stamp here, so
`Conflict` (`Out` here, `In` there, never `seen`) resolves to "the person's
decision stands" — nothing is written or pushed — until a provider reports when
its own list changed.

---

## Lists

```rust
fn reconcile(l: Local, s: Seen, r: Remote) -> Action        // TABLE 1
fn resolve_conflict(t_local: Time, t_remote: Time) -> Side  // TABLE 2
```

```text
TABLE 1 — 3 × 2 × 2 = 12 rows, total
l     s        r        action
Unset Absent   In       AddLocal
Unset Absent   Absent   None              // no local decision, still nothing
Unset In       In       AddLocal
Unset In       Absent   RemoveLocal       // was there, gone now
In    Absent   Absent   PushAdd           // added here, never there  ← the one that matters
In    Absent   In       None              // both agree: in
In    In       In       None
In    In       Absent   RemoveLocal       // removed there
Out   Absent   Absent   None
Out   Absent   In       Conflict          // TABLE 2
Out   In       In       PushRemove        // removed here
Out   In       Absent   None              // both agree: gone
```

```text
TABLE 2 — conflict
t_remote > t_local   -> remote wins  -> AddLocal
t_local  > t_remote  -> local  wins  -> PushRemove
equal                -> local  wins  -> PushRemove
```

Notice for every `RemoveLocal`: `notice("{provider} removed {n} titles")`.

---

## Watched (history)

Monotonic. A provider never un-watches.

```rust
fn on_remote_watched(t, at) {                 // t = movie | episode
    if t.unwatched_local { return }           // local "not watched" marker wins
    t.finished = true;
    t.remote_updated_at = at;
    if t.origin == Local { /* keep origin */ } else { t.origin = Imported(provider) }
}

fn on_local_unwatched(t) { t.finished = false; t.unwatched_local = true }   // blocks import
fn on_local_watched(t)   { t.finished = true;  t.unwatched_local = false }
```

`1970-01-01T00:00:01Z` → `finished = true`, `at = None`, sort last.

---

## Position (resume)

```rust
fn on_remote_position(t, pct, at) {
    if t.finished                 { return }                       // completion dominates
    if t.unwatched_local          { return }
    if pending_for(t.target)      { return }                       // our write is in flight
    if pushed_within(t.target, 5m){ return }                      // our own echo
    if t.origin == Local && t.updated_at > cursor.last_pull_at { return }  // user acted here
    let p = t.duration_ms * pct / 100;                            // pct rounds: a hint, not a bookmark
    t.position_ms = p;
    t.updated_at  = now();                                        // local write clock stays ours
}
```

---

## Echo

```rust
on_delivered(event) { pushed.push({ target: event.target, at: now() }); trim(64, 24h) }
on_import(target)   { if pushed.contains(target, within 5m) { ignore remote stamps for target } }
```

`pushed` lives on the outbox (`TrackingFile`), not on `TrackerState`; it must survive
`prune`, which erases delivered events.

---

## Schedule

```rust
fn tick(now) {
    if !app.running || !provider.enabled.import { return }
    if now < next                               { return }
    if !single_flight.try_lock()                { return }
    let ch = provider.changed(cursor.snapshot);
    if ch.is_empty() { cursor.last_pull = now; next = now + interval(); return }
    match provider.pull(cursor.snapshot, ch) {
        Ok(p)  => { apply(p);                     // idempotent
                    cursor.snapshot = p.snapshot;
                    cursor.list_ids = p.memberships;   // only now
                    cursor.last_pull = now; next = now + interval(); }
        Err(e) => { attempts += 1; next = now + backoff(attempts) }   // 1m, 5m, 30m, 60m
    }
}

fn interval() {
    base = if now - last_local_action < 5m { 60s }
           else if idle > 30m              { 60m }
           else                            { 15m };
    base * jitter(0.9..1.1)                        // one timer for all providers
}
```

Trigger `tick` on: app start, Watch opened, playback ended, mark/list change, timer.

---

## Errors

```text
401                       -> SignInRequired        stop; keep queued events
412 client_id_failed      -> Stop                 (suspended client id)
403 premium_only          -> CapabilityDenied     disable that list kind only
429 rate_limit            -> Retry 1s             ignore Retry-After
429 user_limit_exceeded   -> Wait Retry-After     until reset
400 RATE_LIMIT            -> Retry 5s             write lock, not a quota
400 max_items             -> Split(type → status → original)
5xx / network             -> Backoff
```

---

## Placeholder (imported, not in the library)

```rust
on_import_title_absent(item) { upsert Placeholder { imdb, name, year, poster, available: false } }
rail(continue)               { placeholder rendered greyed  // theme.tiles.grey_uninstalled
                               press -> toast("Not available here") }
on_library_add(title)        { placeholder.available = true; merge into the real item }
on_placeholder { in_no_list && never_watched_locally } { delete }
```

---

## Test cases (TABLE 3)

List rules — one row per TABLE 1 entry, plus conflicts:

```text
l    s       r       t_local  t_remote  ->  expect
U    Absent  In      -        -         ->  local In,  no push
U    Absent  Absent  -        -         ->  no record
U    In      In      -        -         ->  local In,  no push
U    In      Absent  -        -         ->  local Out, notice
I    Absent  Absent  -        -         ->  PushAdd,  local In      // added here
I    Absent  In      -        -         ->  local In,  no push
I    In      In      -        -         ->  nothing
I    In      Absent  -        -         ->  RemoveLocal, notice
O    Absent  Absent  -        -         ->  nothing
O    Absent  In      10m      20m       ->  AddLocal                 // remote newer
O    Absent  In      20m      10m       ->  PushRemove               // local newer
O    Absent  In      10m      10m       ->  PushRemove               // tie -> local
O    In      In      -        -         ->  PushRemove
O    In      Absent  -        -         ->  nothing
```

History / position:

```text
remote watched, local unwatched marker         -> stays unwatched
remote watched, local watched                  -> stays watched, origin kept Local
remote watched at 1970-01-01                   -> finished, no date, sorts last
remote position, local finished                -> ignored
remote position, our push pending              -> ignored
remote position, pushed 2 min ago              -> ignored (echo)
remote position, local written after last pull -> ignored (user acted here)
remote position otherwise                      -> position = duration * pct / 100
```

Schedule / errors:

```text
app closed                          -> no request
provider import switch off          -> no request
nothing changed                     -> exactly 1 request, next tick scheduled
network fails mid-pull              -> snapshot unchanged, same date_from next time
401                                 -> SignInRequired, no retry loop
429 rate_limit                      -> retry ~1 s
429 user_limit_exceeded             -> wait Retry-After
400 RATE_LIMIT                      -> retry ~5 s
```

The same tables, as real `#[test]` cases, are the acceptance gate for S1.
