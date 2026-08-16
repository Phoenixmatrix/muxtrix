# Muxtrix Design Direction

Muxtrix is a native Rust/Iced live-gate-board interface. Its visual language should make fleet and pane state readable at a glance without inventing activity or metadata.

## Foundations

Use the semantic `DesignTokens` in `crates/muxtrix-app/src/main.rs`. Do not introduce ad hoc colors, spacing, or typography where an existing semantic token applies.

The appearance setting supports System, Dark, and Light. System currently resolves to dark, and only the dark appearance is polished. Treat Light as supported configuration, not as a visually complete reference.

The default UI type size is 16 pt and its default weight is Regular. Settings
offers only the weights available for the selected interface family; interface
family and weight changes take effect after restart. Terminal content defaults
to 14 pt, using platform point conversion. Ghostty VT content is rendered
through Iced/wgpu.

Terminal box-drawing and block-element sequences retain fixed grid ownership
without per-cell seams. The complete U+2500..=U+257F box-drawing block renders
as font-independent cell geometry, with every connecting arm reaching the
shared cell edge; full blocks likewise render as exact cell geometry. Other
variable-width Unicode remains isolated so fallback shaping cannot move the
columns that follow it.

Application appearance and terminal color themes are separate controls.
Terminal presets remap semantic terminal defaults and ANSI colors inside
Ghostty; they never recolor explicit RGB output after rendering. The Settings
preview must show the preset background/foreground, selection, cursor, and all
16 ANSI colors before Apply.

Color meaning is fixed:

- Focus: blue, drawn as the focused pane card's accent border and the rail's selection bar.
  Blue also carries the keyboard cursor, which is a proposed focus; the two are
  told apart by form rather than hue (see Layout and hierarchy).
- Warning: amber.
- Success: green.
- Failure: red.

## Layout and hierarchy

The fleet rail is 272 px expanded and 46 px collapsed. The app bar is 44 px
high and carries the workspace tabs as rounded chips; pane headers are 34 px
bands inside their cards. Expanded fleet entries are compact two-line rows on
the rail surface; panes are rounded cards floating on the app field.

Workspaces are enumerated above the Fleet. Each expanded workspace row shows a
rolled-up truthful signal, name, tab/pane counts, and the first available real
branch/current-directory context. Tabs are not repeated under workspace rows:
the fleet already lists them, and the rail stays a workspace-level summary.
The active workspace uses the normal selected surface treatment; collapsed
rows retain workspace identity, number, and state. Workspace creation is a
focused dialog opened from the section add action. Renaming workspaces, tabs,
and panes runs through command-palette actions that open one shared focused
rename dialog; workspace rows carry no inline edit affordances. A pane rename
is an override: clearing it restores the automatic terminal/agent title.

The Fleet scope is explicit. `This` follows the selected workspace without
repeating its workspace name. `All` lists every workspace in session order and
inserts a recessed uppercase workspace band before that workspace's visible
rows; selecting a band switches to that workspace without changing the scope.
The rail header carries a compact This/All scope toggle beside the
Tabs/Agents/Repos projection toggle. Tabs lists every pane in tab order under
its existing tab bands. Agents filters that same order to agent panes only; in
All scope each non-empty workspace remains a separate group, and an entirely
empty result keeps the quiet explanatory state. Repos lists every pane under
the Git repository detected from its live working directory. It merges panes
from different tabs only within one workspace; equal repository names in
different workspaces remain under their respective workspace bands. Panes
outside Git collect in a final `No Repo` band for that workspace. Repos never
nests tab bands: workspace and repository are enough context. Detection is
cached by working directory and runs away from the UI thread so WSL-aware Git
probes cannot interrupt terminal rendering. Each selected segment owns both
the raised fill and its own border. Scope and projection persist as settings
preferences without visiting the settings screen.

Fleet rows show only what is true for their pane. Every expanded row uses two
lines: state signal and linked-worktree name (or repository name, falling back
to the live directory) first; pane title and truthful state second. Agent panes
carry their reported lifecycle. Plain terminal panes use their real terminal
state such as Shell, Starting, Exited, or Unavailable rather than fabricating an
agent lifecycle. Shortcut numbers follow the currently displayed order — and
workspace session order in All scope — in every projection.

The supported minimum window is 720 x 480 logical pixels. The rail collapses
only when the user collapses it — never automatically on resize. Terminal
panes are radius-10 cards on an 8 px gutter grid: the
focused card carries a translucent accent border and drop shadow, a card whose
pane needs a person carries a full amber border and glow (the whole card,
never just an edge), and every other card a hairline. Split handles are the
8 px gaps themselves, showing an accent rule only while dragging. Dense pane headers suppress secondary state copy when the window is
below 1080 px or more than two panes are visible, while the fleet retains the
full state. Text that belongs on one line clips instead of reflowing controls.

Global Attention contains only problems that are not owned by a pane. Omit the section when it is empty. Activity tied to a pane belongs in that pane's fleet context.

Workspace tabs live in the app bar as 29 px rounded chips: signal dot, name,
and a close affordance inside one chip, the active chip on a stronger fill and
border, with a quiet add action after the last chip. The bar's right side
holds the Commands pill — icon, label, and the real palette keycap rendered in
the terminal face — then a divider and the settings icon button. Split actions
stay at pane level in each pane header and its overflow menu.
Tabs scroll horizontally, reorder by drag, and can be dropped onto workspace
rows in the expanded rail. A workspace must retain one tab. Closing the last
pane closes its tab automatically; closing the last tab asks whether to close
the workspace, and the sole application workspace remains protected.

Alt+Arrow moves pane focus by real split geometry. At the layout's right edge
focus wraps to the next tab's first pane, and at the left edge to the previous
tab's last pane, cycling across the tab list; vertical navigation never leaves
the tab. A pane that needs a person — an agent
waiting for input or a terminal with unread attention — draws the full amber
card border and glow in addition to its amber chip and fleet signals, so a
visible pane that needs attention reads instantly.

Ctrl++ grows the focused pane by roughly thirty percent per step and Ctrl+-
walks that pane's resize history backward. Undirected growth follows Zellij's
geometry heuristic: prefer a fully aligned neighbor above, then below, left,
and right; partial edge contact falls back to the pane tree's original local
split. Decrease reverses the boundary chosen by growth even after that boundary
crosses the workspace midpoint. When another pane would be consumed,
its live terminal folds into the same footprint as a title-height sheet: stacked
headers stay in pane order, expose truthful signal/title/state, and carry a
short downward surface shadow so the group reads as layered paper without
turning the workspace ornamental. The focused sheet alone expands; clicking a
header or using Alt+Up/Down focuses and expands that pane.

Alt+[ and Alt+] cycle the active tab through Zellij's default tiled swap set:
Base, Vertical, Horizontal, Stacked, and Half-stacked. A complete cycle restores
the exact Base tree, pane identity, order, and focus. Layout changes never
restart a process. Muxtrix does not expose Zellij's separate floating-pane swap
layouts until it has a floating-pane model of its own.

Pane headers show signal, title, a mono command chip, state, and 24 px icon
controls ordered non-destructive first — splits, maximize, a divider, the
overflow menu, and last the close with a danger hover. The chip names the
program a pane is running; a backend that supplies its own login shell has no
program to name and shows no chip rather than repeating its profile. The title
takes whatever width that chrome leaves on the measured card and truncates
there, bounded so it can never displace the state or the controls. The
header's rounded top corners carry the card radius and one fill spans the
whole band. Focus and attention live on the card border, never on a separate
top rail. Preserve native button semantics, tooltips, keyboard operation, and
visible keyboard focus for every action.

Terminal-emitted window titles replace the pane's fallback shell label and feed
the native window title. Treat this as live terminal metadata: sanitize it,
keep it pane-local, and never let another process rename an unrelated pane.
Harness-owned animated progress glyphs are state decoration rather than title
content: omit them from pane identity and native window chrome while preserving
real task or session copy, so the OS title changes only when its content does.

Text actions use compact 30-32 px control frames with vertically centered
content. Tooltips use the opaque raised panel surface and a strong one-pixel
border so their labels remain readable over terminal content.

Interface type sizes are expressed against a 14 pt reference scale, so changing
the interface size moves headline and secondary copy together by the same
factor. Fleet copy is budgeted from the rail's available width, not from a fixed
character count: trailing state claims its natural width first, then the title
is shaped in the configured interface face and ellipsized to the exact space
left. Secondary budgets that are not width-bound scale inversely with the
interface size. The rail stays 272 px at every interface size; a larger size
truncates sooner rather than demanding a wider rail.

Expanded fleet entries are 52 px two-line rows. The first line carries the
pane's truthful state pip and linked-worktree name, falling back to repository
name and then the live directory. The second line aligns under that identity
and carries the pane title plus a trailing textual state; when an automatic
pane title repeats the first-line worktree/repository/directory identity, the
row spends that lane on truthful activity or command copy instead. Unread count
sits immediately before state when present. Both flexible text lanes are shaped
in the configured interface face and ellipsized to the measured width left
after fixed trailing content claims its space. Direct pane navigation remains
keyboard-only: expanded rows never print Ctrl/Cmd+1 through 9 hints. Workspaces,
tabs, and repositories group under recessed uppercase bands that carry an amber
rollup dot when a visible pane inside needs a person. Repos uses workspace and
repository bands only, with no nested tab grouping. The workspace cards above
the fleet keep their roll-ups: name, state, counts, and the live mono path.

A rail row never changes type weight with its state. Weight is the one property
that alters text metrics, so varying it re-fits the row's own ellipsis and
slides every glyph sideways as the selection moves; state rides on colour, fill,
and the leading mark instead, none of which cost layout.

The rail carries two blue marks at once, and they must never be mistaken for
each other. Where focus actually is: an unbroken 3 px leading bar and the quiet
neutral row fill. Where the prefix keyboard cursor is standing, before Enter
commits it: the same bar cut into rungs, plus an accent row tint, a complete
accent perimeter, and the row's headline — worktree name, workspace name, or
band label — in accent. The cursor is transient and answers a question the eye
is actively asking, so it is allowed to shout where selection stays quiet, and
it wins the row when it lands on the focused one. The pair is distinguished by
form as well as hue: solid against broken, no perimeter against a full one. The
collapsed rail has no room for a rung bar, so the row's numeric identity turns
accent instead, and the `Navigate — ↑↓ move · Enter select · Esc exit` hint
names the mode in words for as long as the cursor exists.

Pane-state signals use green only while an agent is actively working; neutrals
for an ordinary shell, idle agent, completed turn, or stopped process; amber
only for work that needs a person; and red for failure. Focus remains the
separate blue rail, and every pip retains a nearby text state instead of relying
on color alone. A user-issued Ctrl+C is immediate pane-local evidence that a
running turn stopped; the fleet must not remain green while waiting for an agent
harness that may never emit an interruption lifecycle hook.

A pane may also project a *roster* rather than one conversation: Claude Code's
Agents view, reached with `←` on an empty composer or launched directly. Such a
pane is not one agent, so it does not wear one agent's pip. It keeps the same
footprint and the same signal colour drawn as a core inside a ring — a fleet in
a container — in every surface that stands for a single pane: the fleet row, the
pane header, the collapsed rail row, and the stacked title sheet. Tab and
workspace roll-ups are unchanged; they already aggregate and keep the solid dot.
The ring alone is not enough: a hairline outline in the palette's quiet greys
reads as no pip at all, and a healthy fleet — everything finished, nothing
running — is exactly when the roll-up is quietest. Every state keeps a solid
centre, so the row always shows that something is being aggregated there.

A roster row reports the single most important state inside it with its count —
`1 needs input`, `3 working`, `4 completed`, `2 idle`, or `No agents` —
following the same ranking the workspace card uses. That roll-up stays in the
second-line state lane and never creates a third line.
Those counts come from each session's own reported state, never from the
harness's on-screen "awaiting input" tally, which also counts sessions that are
merely idle. The roll-up counts the sessions the view itself lists — the
background ones — because every interactive Claude Code already owns the fleet
row of the pane it runs in, including the pane doing the viewing. Until the
first read lands the row says `Agents`, because the count is genuinely unknown;
a failed read keeps the previous counts rather than inventing new ones, and a
read that has never once succeeded says `Unavailable` with the reason on the
activity line rather than leaving a row that waits forever.

Titles that name the harness's current view rather than its work — the roster's
own title and the `current session` label Claude Code emits on the way back —
are chrome, not identity. They never rename a fleet row, so toggling views
leaves a pane's earned name intact.

The collapsed fleet rail keeps pane identity as well as state: each ruled row
shows its numeric pane identity and signal without shortcut notation, while its
tooltip exposes the full title, state, and latest truthful activity.

Only installed monospaced families appear in Terminal text and history. Never offer
a named font that would silently fall back to proportional text; unavailable
saved choices recover to System monospace and surface a global warning.

Show process and agent state truthfully. A live interactive terminal has no
agent lifecycle: its fleet entry uses `Shell` as its surface state and reports
real launch states such as `Starting`, `Exited`, and `Unavailable`. It never
borrows agent lifecycle copy; unread terminal attention uses `Needs input`
beside an amber pip. Agent states use sentence case. Show cwd and session
metadata only when those values are real. Never use placeholder status to
fill visual space.
For Codex and Claude Code, reserve `Needs input` for positive evidence in the
current live agent UI. Permission hooks that may still be handled by an
automatic reviewer are not human-attention evidence.

## Primary actions and navigation

Commands is a top action and opens with `Ctrl/Cmd+P`. Settings is a top action
and opens with `Ctrl/Cmd+,`. `Ctrl/Cmd+1` through `Ctrl/Cmd+9` navigates directly
to panes; adding Shift switches directly to the workspace in the same numbered
session-order position.

The status bar is optional and defaults to off.

The command palette uses a clearly bordered accent-tinted selected row. Arrow
keys and Tab/Shift+Tab move the selection while Enter executes it; shortcut
labels keep a dedicated right gutter from the scrollbar.

Settings follows a compact native preference-table grammar: section headings
sit outside continuous bordered row groups; each row pairs a left label and
secondary description with a compact right-side control; dividers replace
detached cards. Buttons are 30 px high with explicit primary, secondary, and
destructive roles, plus a quiet role for navigation that happens to be a
button: no fill or border at rest, a real surface only under the pointer, so
moving between surfaces never competes with the actions on them.

Settings owns the full application window. Its persistent 52 px top bar reads
left to right as the quiet return to the terminal, a rule, the `Settings`
title, then — anchored at the trailing edge — the Preferences/Worktrees
switch, drawn as the same recessed-well segmented toggle the fleet heading
uses rather than a second selection idiom. The bar's insets land its leading
glyph and trailing edge on the page's own content margin. Every fixed-height
band in settings centers its content: a band that sets a height without
centering renders flush against its top edge and clips the controls inside it.

Preferences keeps Cancel and the accent Apply action in a persistent footer
while its content scrolls independently. Worktrees is a ruled, responsive
inventory rather than a modal: identity, branch, protection/usage,
local-only-commit risk, and actions have distinct lanes; repository discovery
and per-checkout Git inspection happen off the UI thread behind an immediate
loading state.

Preferences ends with a Versions group for the Muxtrix window and its local
control service. Each row keeps the active build primary and the installed
binary comparison secondary. The disk probe runs off the UI thread against the
package-managed launch path retained at startup, so replacing an executable
while this window remains open produces an amber, textual restart notice
instead of silently reporting the old inode as current.

The inventory's lanes are derived once from the window's real width and shared
by the header, every row, and the ellipsis budgets, so the three cannot drift
apart. The three trailing lanes hold bounded copy and stay fixed; identity and
branch are the unbounded strings and split whatever width is left, so a wider
window widens the copy rather than wrapping a long branch inside a fixed box.
Every string is budgeted against the lane that holds it and ellipsized inside
it — a row is one line tall at every width, and copy never slides under the
next lane to be cut mid-glyph. A checkout's name is already the last segment
of its path, so the secondary line carries the directory that holds it instead
of printing the leaf twice.

The action lane names one action and keeps that label in every row. A row that
cannot be removed shows the same button disabled, never relabelled with its
reason: the reason is state, it already reads in the status lane one lane to
the left, and relabelling printed the same words twice in one row while making
the lane's shape change from row to row. A tooltip repeats the reason so
hovering the disabled control still explains itself.

The inventory is keyboard-operable, and its footer states that grammar with
the real keys drawn as keycaps in the terminal face: Up/Down move the
selection, Delete removes an eligible checkout, Escape returns to the
terminal and discards the settings draft. Advertising a key obliges the page
to receive it: settings surfaces that own navigation keys must claim them
before the generic non-workspace key handling consumes them.

Terminal scrollbars are three-pixel hover-only overlays with a twelve-pixel
invisible mouse target. The thumb supports track clicks and dragging, never
reserves layout width, and never changes the terminal grid. Text-selection
anchors use scrollback rows so highlights follow their content while the
viewport moves. Pane context menus float above the terminal and must never push
its content or trigger a resize. Menu rows group by intent — clipboard, then
layout, then lifecycle — separated by inset one-pixel dividers, with the
destructive close row in the danger color. Rows that cannot act stay present
but disabled in place so positions never shift with invisible state, and
clipboard rows carry their shortcut hints. Because rows sit on the raised
panel, their hover fill uses the line token, never the panel's own color.

Clipboard bindings follow Ghostty's defaults: Ctrl+Shift+C/Ctrl+Shift+V, and
Cmd+C/Cmd+V on macOS. Bare Ctrl+C and Ctrl+V always belong to the terminal.
A genuine terminal text-selection drag copies to the system clipboard once,
when the left button is released; click jitter and pointer moves never replace
clipboard contents. Mouse-reporting applications retain unmodified pointer
input, while Shift explicitly restores local selection. Paste is encoded
against the terminal's own bracketed-paste state; the chords are surfaced in
both the context menu and the command palette.

Terminal rows use fixed-height, fixed-column projection. Unicode fallback
glyphs that may have variable natural metrics stand in isolated runs; wide
glyphs own two columns and their spacer tails own none. Animated agent
indicators must never move adjacent text or alter another row's baseline, and
all glyph output remains clipped to the pane in split and unsplit layouts.

## GitHub review ledger

The focused repository's GitHub surface is a 372 px review ledger on the right,
never a dashboard or detached card stack. At 1080 px and wider it docks beside
the workspace with one strong vertical divider and no shadow. Below that width
it becomes an opaque right-aligned overlay at the same width, separated from
the terminal by a restrained shadow cast only to the left. Opening, closing,
refreshing, or scrolling the ledger must not disturb terminal geometry.

The ledger separates two different truths with one recessed Local/Pull requests
switch. Local is the focused pane's working tree and never requires GitHub
authentication. Pull requests is the repository's remote inventory. The active
tab alone owns Refresh, loading, errors, scrolling, and selection; switching
tabs must not make local edits look like pull-request changes or vice versa.
Repository and branch identity stay fixed above the switch.

Local refreshes when the panel opens, when Refresh is used, and once after pane,
tab, or workspace focus moves to another local pane. Focus queues one
asynchronous Git read; it does not introduce a repeating timer. A repository
change replaces both tab caches, while movement within one repository updates
only local state. Pull requests load when their tab is first opened, when
explicitly refreshed, or when a supported harness reports that the focused
pane's agent completed a turn. Turn completion invalidates a hidden cached
list; when Pull requests is visible, it replaces that list immediately behind
the existing loading state. Compaction and handoff maintenance do not count as
turn completion.

The pull-request list fetches lightweight identity and readiness summaries,
then searches title, number, author, head, and base locally. Continuous 58 px
rows keep title, open/draft state, and a semantic readiness icon above identity
and branch metadata. Hovering the icon exposes the same concise readiness label
and explanation used by detail view; its shape and text keep status legible
without relying on color. The search keeps a visible `Search` label above the
field; placeholder text is guidance, never the field's only
name. Render only visible rows plus bounded overscan. Filtering resets the list
to its start, and filtering or refresh clamps both scroll offset and keyboard
cursor to the new result set so virtualization never produces a blank viewport
from stale position. Selecting a pull request replaces the list with its
readiness, identity, checks, and paginated changed files. Only that selection
fetches full metadata and patches, so repositories with many open pull requests
do not pay the cost of every file list. A quiet back row returns to the
searchable list.
Readiness always pairs its semantic signal with a concise text label and
explanation. Green means ready or passed, amber means pending or blocked on
human or remote work, red means failed or conflicting, and neutral means
unknown or draft; color is never the only carrier of meaning.

The fleet footer owns GitHub account state even when the ledger is closed. Its
expanded form shows account or connection copy with a state dot; its collapsed
form keeps the GitHub mark and dot, with the full state available in a tooltip.
The mark and its copy carry the same weight as the rest of the rail rather than
a hairline scale of their own. The dot belongs to the account rather than to the
rail's edge, so the name hugs its own copy and the dot follows it; the name is
shaped and ellipsized against the width the footer's remaining anatomy leaves it
— the mark, the two gaps, the dot, and the collapse control — so a long account
name stops inside its own lane instead of running under the dot or pushing
collapse off the rail's edge.
When authentication is unavailable or in progress, only Pull requests replaces
its body with one centered explanation and one truthful next action; Local
remains usable. Successful browser authentication refreshes the pull-request
surface rather than introducing a second flow.

A full load owns the active tab body. Initial loading and manual refresh replace
its evidence and actions with one centered 3 x 3 dot activity mark and truthful
Local, pull-request-list, or selected-pull-request copy. The repository, branch,
and tab switch remain for context, but the switch is disabled; Close remains
available and Refresh disappears until replacement state lands. The dots
animate inside a fixed footprint without moving adjacent copy. There is no
window-focus polling or background pull-request probe.

Keyboard ownership is explicit and opt-in. Merely opening the ledger never
claims terminal keys; interacting with a ledger tab, search field, row, or file
list moves keyboard ownership into the panel, and focusing a terminal moves it
back out. While the ledger owns focus, Tab and Shift+Tab traverse only the
controls present in the active tab or detail view, arrows move the active list,
Enter activates its cursor, and Escape steps back or releases panel focus. A
text field keeps ordinary editing keys. Panel navigation keys are handled only
while that ownership is present, so an open ledger cannot steal shell input.

Changed files use continuous 42 px ruled rows under a compact summary band.
Render only the visible rows plus bounded overscan. Each row keeps its path and
secondary status on the left, then fixed monospaced addition and deletion lanes
on the right. The path truncates before those lanes; counts never move or wrap.
Use success for additions, danger for deletions or conflicts, and neutral copy
for ordinary modifications.

Selecting a local or pull-request file opens the same full-height unified diff
while the ledger stays visible in its originating tab for file navigation. Back
and Escape return to the workspace; returning from a PR detail returns to its
searchable list.
When GitHub omits an inline patch for a large textual file, opening that file
may reconstruct its diff lazily from the exact base and head blobs; never add
that cost to pull-request or file-list loading. Bound each source download
before buffering it, write any comparison inputs to owner-only temporary
storage, retain the viewer's existing truncation limits, and distinguish
binary, oversized, network-error, and retry states with truthful copy.
The diff's code lane is measured in configured terminal cells after both line
number gutters and their spacing. At 80 columns or wider, logical lines wrap at
glyph boundaries inside the available lane and horizontal scrolling disappears;
continuations keep the semantic line background but no repeated line numbers.
Below 80 columns, lines remain intact behind horizontal scrolling because
wrapping would fragment the code. Both modes retain bounded vertical
virtualization, and resizing across the threshold anchors the same logical line.

Draft state is a reversible secondary action in pull-request detail. A draft
offers `Mark ready`; a reviewable pull request offers `Convert to draft`.
Updating state keeps the detail visible, disables conflicting actions, and
updates both readiness and the list status without requiring a manual refresh.

Merge is a guarded inline action, not a modal interruption. Enable it only when
GitHub reports a clean, mergeable pull request whose checks and review
requirements are satisfied. The confirmation stays inside the pull-request
block, names the pull request, says that the branch is kept, and presents
Cancel beside the explicit merge action. Confirm against the displayed head
commit so refreshed remote state cannot turn stale readiness into an unreviewed
merge.

## Extension rules

New UI must:

1. Reuse semantic `DesignTokens` and preserve the established state colors.
2. Keep the 272/46 px rail states and 44 px header rhythm unless the entire layout contract is deliberately revised.
3. Extend continuous fleet rows instead of introducing card styling.
4. Place alerts according to ownership: global only for non-pane problems, pane-bound otherwise.
5. Render only truthful process, agent, cwd, and session data.
6. Preserve native control semantics, tooltips, keyboard operation, and full-perimeter focus treatment.
7. Prefix navigation: Ctrl+G arms a one-shot Zellij-style layer (announced
   by persistent bottom-center guidance), then `w` starts a rail walk at the
   workspaces and `f` at the first visible fleet entry; arrows move the cursor
   through visible workspaces, tab bands, and pane rows in visual order, Enter
   activates, Esc exits. Unrecognized keys are consumed but do not dismiss
   either mode. The cursor uses an accent-tinted fill with a complete 1 px
   accent perimeter; actual selection keeps its neutral fill and 3 px leading
   bar, so the proposed destination is distinct from the current location.
7. Add interface icons as SVG assets under `crates/muxtrix-app/assets/icons`.
8. Respect the default-off status bar and the established command shortcuts.
9. Keep compact-window behavior and terminal-grid resize coalescing intact;
   dense pane headers consolidate actions into overflow without removing them.

## Key paths

- UI implementation and tokens: `crates/muxtrix-app/src/main.rs`
- SVG icon assets: `crates/muxtrix-app/assets/icons`
- Surface brief: `docs/DESIGN_SURFACE_APP_SHELL.md`
