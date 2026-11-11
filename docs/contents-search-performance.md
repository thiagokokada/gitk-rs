# Content search workers

`contents:` search uses long-lived `git diff-tree --stdin` processes. Set
`GITK_SEARCH_WORKERS` to an integer from 1 through 8 to change the pool size;
unset or invalid values use 2. Path-only searches use one process.

Batches of up to 64 metadata candidates are assigned round-robin and results
are drained in submission order. The existing session-wide candidate limit
bounds pending work, rather than multiplying that limit by the worker count.
Each worker retains bounded input/output channels and owns cancellation and
child cleanup. Idle steps sleep briefly; steps that send or receive work do not.
