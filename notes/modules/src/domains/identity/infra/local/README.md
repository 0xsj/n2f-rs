# Local adapters: counting refusals, and what a full store must not do

**Origin:** implementing L01–L03, B01–B02 and M01 spec-first on 2026-09-12.

The limiter increments both buckets on every call, permitted or refused, and
takes the larger remaining window as its retry-after. Counting refusals is what
makes a refused subject also consume its source's budget, which the spec checks by
switching subjects on one source; the mutation that skips the source bucket is
caught by that scenario and by the state test, which finds only one key.

State is keyed by the keyed digest of `operation || 0 || subject`, never the
subject, so a dump of the map cannot yield an email or a source address. The key
bound is enforced only when a new key would be inserted: expired windows are
evicted first, and a store that is still full refuses with the dependency failure
rather than permitting. A permit on overflow would be the fail-open A16 forbids,
and the mutation that returns it is caught.

Operations without a configured policy are refused as a dependency failure rather
than permitted, on the same reasoning; the defaults cover the seven operations
the application uses.

The blocklist folds entries and candidates the same way, NFC then lowercase, and
compares whole strings. A substring comparison would reject every password that
contains a blocked word, which the spec forbids by allowing a superset of an entry.
`str::to_lowercase` is full Unicode lowercasing, close to but not identical with
simple case folding; the difference does not affect ASCII lists.

**Limits:** process-local, per instance, lost on restart; not the stage 7 limiter.
The mail adapter delivers nothing and only counts.

**Used in:** src/domains/identity/infra/local/mod.rs and tests/identity_local_spec.rs.
