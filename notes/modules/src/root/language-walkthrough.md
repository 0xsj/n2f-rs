# Process composition: implementation walkthrough

Configuration is validated before generating process identity or constructing
logging resources. DEMO_TOKEN has no implicit default, including when output is
disabled. Only the named foundations command runs this example; simulated
Unavailable/Conflict/unknown failures do not claim real adapter calls.

The operation owner permits one retry because the fixture performs no external
write. Unavailable alone is not a reusable retry policy. Startup, prepared work,
first execution and retry retain distinct lifetimes. Root owns the logger close
deadline and reports safe bootstrap or delivery failures.

tools/verify_foundations.py builds this clone's actual command and checks eleven
process scenarios. It parses JSON relationships and redaction, checks no-op/severity,
invalid-config exit, empty values, and runs real pseudo-terminals for auto-color.
Each clone carries the same checks without a sibling dependency.

## Rust mechanics

? keeps setup failures in Result. Arc<SystemClock> supplies a shared clock to the
logger worker boundary without a global singleton. The demo uses a local RefCell
around its generation closure so the factory and work preparation can borrow it
sequentially; no RefCell crosses a thread boundary. Nested borrow misuse would panic,
so the root keeps those calls synchronous and nonreentrant.

std::io::IsTerminal captures output capability. The command owns exit status; a
successful demo's handled failure examples still return 0. Returning from close
does not imply a blocked native writer syscall was canceled.

