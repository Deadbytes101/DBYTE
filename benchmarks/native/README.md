# DByte native performance corpus

These sources are intentionally compatible with v12.3.0. The v12.4 verifier
must compile the exact same bytes with two independent peeled-tag exports and
with the candidate compiler. Compilation and process startup are reported
separately from measured execution.

The manifest is the authority for warmups, sample count, stability and release
thresholds. A workload name is never exposed to the compiler and no source-name
or content-specific optimization is permitted.
