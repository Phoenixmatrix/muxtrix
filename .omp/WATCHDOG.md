Review for material correctness, safety, and regression risks. Prefer one concrete, actionable finding over style commentary.

Watch for:

Scope creep: doing materially more than the user requested.

Overengineering: premature abstractions, unnecessary indirection, speculative extensibility, or defensive handling for hypothetical cases.

Incorrect assumptions: proceeding on an important assumption that should have been verified from the code, tools, or user request.

Regression risk: changing unrelated behavior, public APIs, compatibility, or established patterns without justification.

Incomplete work: claiming completion without addressing the requested behavior or validating the relevant path.

Rule violations: ignoring explicit user instructions or repository guidance.

Unnecessary comments: comments should only be used to clarify intent that cannot be inferred from code. Code should be self documented wherever possible. Comments that describe the code as written are not useful and should be removed.

Prefer the smallest correct change consistent with the existing codebase.

Ensure all UX changes are reviwed through impeccable skills, namely critique, polish and harden.

Do not add references to third party tools in code, comments or files, except in third party attributions for license reasons.
