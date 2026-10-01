Review and simplify the requested code while preserving observable behavior.

Determine the scope before editing. Follow the repository's instructions and use its version control system. Unless the user supplies paths, a revision range, or another scope, inspect staged and unstaged changes and relevant untracked source files. If the working tree is clean, inspect the most recent commit. Keep the review within that scope; inspect surrounding callers and existing utilities as needed. If there is no reviewable change, report that and stop.

Launch three independent review agents in parallel using the available subagent tool. Give each the same scope, change summary, relevant repository rules, and access to the diff. For a large diff, provide file or revision references rather than silently truncating it. The reviewers report findings and do not edit files:

1. Reuse: look for duplicated logic, helpers that already exist, and opportunities to use an established abstraction instead of adding another one. Identify concrete existing code to reuse. Avoid extracting abstractions for merely similar code with different responsibilities.
2. Quality: look for redundant state, unnecessary indirection, excessive parameters, copy-pasted variants, deeply nested logic, and abstractions that make the changed code harder to understand. Prefer the clearest solution consistent with the project's conventions, not the fewest lines.
3. Efficiency: look for repeated work, unnecessary reads or allocations, serial independent operations, overly broad queries, resource leaks, and expensive work added to frequently used paths. Require a concrete reason the change matters; avoid speculative optimization.

Each reviewer should return actionable findings with file locations, evidence, and a bounded proposed fix, or explicitly report no findings. If parallel delegation is unavailable, disclose that limitation and perform the three reviews sequentially without claiming to have launched agents.

Wait for all three results. Resolve disagreements, remove duplicate findings, and verify each candidate against the actual code and its callers. Apply the worthwhile, behavior-preserving fixes yourself; skip false positives and changes whose cost or risk exceeds their benefit. Preserve unrelated user edits. A simplify request authorizes local improvements, not commits, pushes, publishing, or new features.

Run the focused checks required by the repository for the affected code. Inspect the final diff for accidental behavior changes and scope expansion. Finish with a concise summary of the improvements and validation, or state that no worthwhile simplifications were found. Make no claim of three-agent review unless all three reviews actually completed.
