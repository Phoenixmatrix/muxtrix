// The capture matrix: every Muxtrix state the headless harness can reach,
// with the review question each frame is meant to answer.

const THEMES = [
  ["muxtrix-dark", "Muxtrix Dark", false],
  ["ghostty", "Ghostty Default", false],
  ["tokyo-night", "Tokyo Night", false],
  ["catppuccin-mocha", "Catppuccin Mocha", false],
  ["dracula", "Dracula", false],
  ["gruvbox-dark-hard", "Gruvbox Dark Hard", false],
  ["git-hub-dark-default", "GitHub Dark", false],
  ["nord", "Nord", false],
  ["rose-pine", "Rosé Pine", false],
  ["kanagawa-wave", "Kanagawa Wave", false],
  ["solarized-dark", "Solarized Dark", false],
  ["atom-one-dark", "Atom One Dark", false],
  ["monokai-pro", "Monokai Pro", false],
  ["catppuccin-latte", "Catppuccin Latte", true],
  ["git-hub-light-default", "GitHub Light", true],
  ["rose-pine-dawn", "Rosé Pine Dawn", true],
  ["solarized-light", "Solarized Light", true],
  ["tokyonight-storm", "Tokyo Night Storm", false],
  ["catppuccin-macchiato", "Catppuccin Macchiato", false],
  ["catppuccin-frappe", "Catppuccin Frappé", false],
  ["gruvbox-dark", "Gruvbox Dark", false],
  ["everforest-dark", "Everforest Dark", false],
  ["snazzy", "Snazzy", false],
  ["night-owl", "Night Owl", false],
  ["palenight", "Palenight", false],
  ["zenburn", "Zenburn", false],
  ["monokai", "Monokai", false],
  ["rose-pine-moon", "Rosé Pine Moon", false],
  ["kanagawa-dragon", "Kanagawa Dragon", false],
  ["git-hub-dark-dimmed", "GitHub Dark Dimmed", false],
  ["ayu-mirage", "Ayu Mirage", false],
  ["ubuntu", "Ubuntu", false],
  ["tomorrow-night", "Tomorrow Night", false],
  ["gruvbox-light", "Gruvbox Light", true],
  ["one-half-light", "One Half Light", true],
  ["tomorrow", "Tomorrow", true],
];

/** @type {Array<{slug: string, title: string, group: string, capture: string, viewport?: string, settings?: object, check: string}>} */
const cases = [];

const add = (entry) => {
  cases.push({ viewport: "1280x800", ...entry });
};

// ── Workspace shell ─────────────────────────────────────────────────────────
add({
  slug: "workspace-default",
  title: "Workspace",
  group: "Workspace shell",
  capture: "-",
  check:
    "Two split panes, the workspace rail, tab strip and status footer at the default 1280×800.",
});
add({
  slug: "workspace-wide",
  title: "Workspace · 1920×1080",
  group: "Workspace shell",
  capture: "-",
  viewport: "1920x1080",
  check: "Rail and pane chrome should not stretch or leave the tab strip stranded at desktop width.",
});
add({
  slug: "workspace-large",
  title: "Workspace · 1600×1000",
  group: "Workspace shell",
  capture: "-",
  viewport: "1600x1000",
  check: "Intermediate width: the GitHub panel threshold is 1080, so the shell is in its docked mode.",
});
add({
  slug: "workspace-medium",
  title: "Workspace · 1024×680",
  group: "Workspace shell",
  capture: "-",
  viewport: "1024x680",
  check: "Below the 1080 docking threshold. Rail density should still be the roomy variant.",
});
add({
  slug: "workspace-compact",
  title: "Workspace · 820×560",
  group: "Workspace shell",
  capture: "-",
  viewport: "820x560",
  check: "Compact density: pane headers drop to the tight control set and the rail narrows.",
});
add({
  slug: "workspace-minimum",
  title: "Workspace · 720×480 (minimum)",
  group: "Workspace shell",
  capture: "-",
  viewport: "720x480",
  check: "The smallest supported window. Nothing may overlap, clip, or fall out of the frame.",
});
add({
  slug: "sidebar-collapsed",
  title: "Rail collapsed",
  group: "Workspace shell",
  capture: "fleet-tabs-collapsed",
  check: "Collapsed rail keeps workspace pips legible and hands the width back to the panes.",
});
add({
  slug: "maximized-pane",
  title: "Maximized pane",
  group: "Workspace shell",
  capture: "maximized-pane",
  check: "One pane fills the content area; the rail still lists every pane in the tab.",
});
add({
  slug: "global-alert",
  title: "Global attention alerts",
  group: "Workspace shell",
  capture: "global-alert",
  check: "Two alerts in the rail's Attention section, with agent identity and body legible.",
});
add({
  slug: "prefix-armed",
  title: "Prefix mode armed (Ctrl+G)",
  group: "Workspace shell",
  capture: "prefix-armed",
  check: "The keyboard-mode guidance pill sits bottom-centre and does not cover terminal content.",
});
add({
  slug: "rail-nav",
  title: "Rail keyboard navigation",
  group: "Workspace shell",
  capture: "rail-nav",
  check: "Navigation cursor highlight must read as distinct from the focused-pane highlight.",
});
add({
  slug: "rail-nav-collapsed",
  title: "Rail keyboard navigation · collapsed",
  group: "Workspace shell",
  capture: "rail-nav-collapsed",
  check: "The cursor has to stay findable at 46px, where the row has no title to tint.",
});
add({
  slug: "toast",
  title: "Confirmation toast",
  group: "Workspace shell",
  capture: "toast",
  check: "Transient pill, bottom-centre, quiet enough not to compete with the terminal.",
});

// ── Fleet ───────────────────────────────────────────────────────────────────
add({
  slug: "fleet-tabs",
  title: "Fleet · Tabs",
  group: "Fleet",
  capture: "-",
  check: "Default projection: tab bands with their panes nested underneath.",
});
add({
  slug: "fleet-agents",
  title: "Fleet · Agents",
  group: "Fleet",
  capture: "fleet-agents",
  check:
    "Flat agent list across tabs. Long agent names must ellipsize without growing the row height.",
});
add({
  slug: "fleet-agents-empty",
  title: "Fleet · Agents (empty)",
  group: "Fleet",
  capture: "fleet-agents-empty",
  check: "The empty state should explain itself rather than showing a blank rail.",
});
add({
  slug: "fleet-agents-collapsed",
  title: "Fleet · Agents collapsed",
  group: "Fleet",
  capture: "fleet-agents-collapsed",
  check: "Collapsed rail must still surface a waiting agent's attention state.",
});
add({
  slug: "fleet-repos",
  title: "Fleet · Repos",
  group: "Fleet",
  capture: "fleet-repos",
  check: "Panes grouped by checkout, with a 'No repo' group for shells outside a repository.",
});
add({
  slug: "fleet-repos-compact",
  title: "Fleet · Repos · 820×560",
  group: "Fleet",
  capture: "fleet-repos",
  viewport: "820x560",
  check: "Repository group headers at compact density.",
});
add({
  slug: "agent-lifecycle-states",
  title: "Agent lifecycle colours",
  group: "Fleet",
  capture: "agent-lifecycle-states",
  check: "Failed, Stopped and Idle in one frame — each state's pip and label must be distinguishable.",
});
add({
  slug: "agents-roster",
  title: "Claude agents roster roll-up",
  group: "Fleet",
  capture: "agents-roster",
  check: "The hollow roll-up pip must be visibly different from the neighbouring solid lifecycle pip.",
});
add({
  slug: "needs-input",
  title: "Agent needs input",
  group: "Fleet",
  capture: "needs-input",
  check: "Whole-pane amber treatment for a waiting agent, and the matching rail row.",
});
add({
  slug: "pane-attention",
  title: "Unread pane attention",
  group: "Fleet",
  capture: "pane-attention",
  check: "Unread count on an unfocused pane; focusing it elsewhere must not have cleared it.",
});
add({
  slug: "many-tabs",
  title: "Eight tabs",
  group: "Fleet",
  capture: "many-tabs",
  check: "Tab strip overflow: check the last tab's close affordance against the Commands button.",
});
add({
  slug: "many-tabs-compact",
  title: "Eight tabs · 820×560",
  group: "Fleet",
  capture: "many-tabs",
  viewport: "820x560",
  check: "Tab strip overflow at compact width.",
});
add({
  slug: "many-workspaces",
  title: "Five workspaces",
  group: "Fleet",
  capture: "many-workspaces",
  check: "Workspace list growth: rail scroll behaviour and the fleet's share of the height.",
});

// ── Pane layouts ────────────────────────────────────────────────────────────
add({
  slug: "four-panes",
  title: "Four panes",
  group: "Pane layouts",
  capture: "four-panes",
  check: "Nested splits: every pane's grid must match the box it is drawn into.",
});
add({
  slug: "four-panes-compact",
  title: "Four panes · 820×560",
  group: "Pane layouts",
  capture: "four-panes",
  viewport: "820x560",
  check: "Four panes at compact density — headers must stay readable.",
});
add({
  slug: "layout-vertical",
  title: "Layout · Vertical",
  group: "Pane layouts",
  capture: "layout-vertical",
  check: "Even vertical columns.",
});
add({
  slug: "layout-horizontal",
  title: "Layout · Horizontal",
  group: "Pane layouts",
  capture: "layout-horizontal",
  check: "Even horizontal rows.",
});
add({
  slug: "layout-stacked",
  title: "Layout · Stacked",
  group: "Pane layouts",
  capture: "stacked-layout",
  check: "Collapsed pane headers stack with one expanded pane below.",
});
add({
  slug: "layout-half-stacked",
  title: "Layout · Half-stacked",
  group: "Pane layouts",
  capture: "layout-half-stacked",
  check: "One full pane beside a stack of collapsed headers.",
});
add({
  slug: "layout-stacked-compact",
  title: "Layout · Stacked · 820×560",
  group: "Pane layouts",
  capture: "stacked-layout",
  viewport: "820x560",
  check: "Stacked headers at compact density.",
});
add({
  slug: "pane-menu",
  title: "Pane overflow menu",
  group: "Pane layouts",
  capture: "pane-menu",
  check: "Popover anchoring, item spacing, and that it does not clip the pane edge.",
});
add({
  slug: "pane-menu-compact",
  title: "Pane overflow menu · 820×560",
  group: "Pane layouts",
  capture: "pane-menu",
  viewport: "820x560",
  check: "The popover must stay inside the window at compact width.",
});

// ── Command palette & dialogs ───────────────────────────────────────────────
add({
  slug: "palette",
  title: "Command palette",
  group: "Dialogs",
  capture: "palette",
  check: "Selected row, shortcut column alignment, and scrim contrast.",
});
add({
  slug: "palette-query",
  title: "Command palette · filtered",
  group: "Dialogs",
  capture: "palette-query",
  check: "Filtering to 'work' — matching rows only, selection on a real result.",
});
add({
  slug: "palette-empty",
  title: "Command palette · no matches",
  group: "Dialogs",
  capture: "palette-empty-query",
  check: "Empty result state rather than a collapsed, ambiguous panel.",
});
add({
  slug: "palette-compact",
  title: "Command palette · 720×480",
  group: "Dialogs",
  capture: "palette",
  viewport: "720x480",
  check: "The palette must fit the minimum window without clipping its list.",
});
add({
  slug: "workspace-create",
  title: "Create workspace",
  group: "Dialogs",
  capture: "workspace-create",
  check: "Name field, submit affordance, and dialog width.",
});
add({
  slug: "rename-workspace",
  title: "Rename workspace",
  group: "Dialogs",
  capture: "rename-workspace",
  check: "Dialog copy must name what is being renamed.",
});
add({
  slug: "rename-tab",
  title: "Rename tab",
  group: "Dialogs",
  capture: "rename-tab",
  check: "Same dialog, tab target.",
});
add({
  slug: "rename-pane",
  title: "Rename pane",
  group: "Dialogs",
  capture: "rename-pane",
  check: "Same dialog, pane target.",
});
add({
  slug: "close-workspace",
  title: "Close workspace confirmation",
  group: "Dialogs",
  capture: "close-workspace",
  check: "Destructive confirmation: the consequence must be stated, not implied.",
});
add({
  slug: "session-picker",
  title: "Session picker (startup)",
  group: "Dialogs",
  capture: "session-picker",
  check: "Live and dead sessions, ages, pane counts, and the 'start new' escape hatch.",
});
add({
  slug: "session-picker-error",
  title: "Session picker · error",
  group: "Dialogs",
  capture: "session-picker-error",
  check: "Registry read failure — the error must not leave an empty, actionless dialog.",
});

// ── Worktrees ───────────────────────────────────────────────────────────────
add({
  slug: "worktree-dialog",
  title: "New worktree prompt",
  group: "Worktrees",
  capture: "worktree-dialog",
  check: "Base directory, derived path preview, and name field.",
});
add({
  slug: "worktree-manager",
  title: "Settings · Worktrees",
  group: "Worktrees",
  capture: "worktree-manager",
  check: "Long branch names and 'used by' pane titles must truncate, not wrap the row.",
});
add({
  slug: "worktree-switcher",
  title: "Worktree switcher dialog",
  group: "Worktrees",
  capture: "worktree-switcher",
  check: "The restart-in-worktree picker as a modal rather than a settings page.",
});
add({
  slug: "worktree-restart-confirmation",
  title: "Worktree restart confirmation",
  group: "Worktrees",
  capture: "worktree-restart-confirmation",
  check: "Destructive restart confirmation inside the picker.",
});
add({
  slug: "worktree-manager-error",
  title: "Worktrees · delete failed",
  group: "Worktrees",
  capture: "worktree-manager-error",
  check: "Error banner placement relative to the list.",
});
add({
  slug: "worktree-manager-loading",
  title: "Worktrees · loading",
  group: "Worktrees",
  capture: "settings-worktrees-loading",
  check: "Loading state should hold the layout instead of collapsing it.",
});
add({
  slug: "worktree-manager-no-repo",
  title: "Worktrees · not a repository",
  group: "Worktrees",
  capture: "worktree-manager-no-repo",
  check: "Explains why there is nothing to manage.",
});

// ── GitHub panel ────────────────────────────────────────────────────────────
add({
  slug: "github-panel",
  title: "Repository panel · local changes",
  group: "GitHub",
  capture: "github-panel",
  check: "The Local tab is selected and shows only the focused pane's working-tree changes.",
});
add({
  slug: "github-panel-floating",
  title: "GitHub panel · floating (1024×680)",
  group: "GitHub",
  capture: "github-panel",
  viewport: "1024x680",
  check: "Below 1080 the panel floats over the workspace — check the scrim and right alignment.",
});
add({
  slug: "github-pull-requests",
  title: "Repository panel · pull requests",
  group: "GitHub",
  capture: "github-pull-requests",
  check: "The Pull requests tab presents a searchable, compact open-PR inventory with stable title, author, branch, and state lanes.",
});
add({
  slug: "github-pull-request-search",
  title: "Pull requests · filtered",
  group: "GitHub",
  capture: "github-pull-request-search",
  check: "Searching by title narrows the list and reports the matched count without moving the search field.",
});
add({
  slug: "github-pull-requests-scrolled",
  title: "Pull requests · virtualized tail",
  group: "GitHub",
  capture: "github-pull-requests-scrolled",
  check: "The 120-item virtualized list reaches its tail without blank rows or layout shifts.",
});
add({
  slug: "github-scrolled",
  title: "Pull request · changed files scrolled",
  group: "GitHub",
  capture: "github-scrolled",
  check: "The selected PR's virtualized changed-file list reaches its tail without blank rows.",
});
add({
  slug: "github-diff",
  title: "GitHub · unified diff",
  group: "GitHub",
  capture: "github-diff",
  viewport: "1440x800",
  check: "At a width that holds at least 80 terminal cells, long logical lines wrap without a horizontal scrollbar while line numbers and add/delete/hunk treatments remain aligned.",
});
add({
  slug: "github-diff-minimum",
  title: "GitHub diff · minimum window (720×480)",
  group: "GitHub",
  capture: "github-diff",
  viewport: "720x480",
  check: "At minimum width the compact diff header stays inside its surface while the file panel remains usable.",
});
add({
  slug: "github-diff-horizontal",
  title: "GitHub diff · below wrap threshold (1024×680)",
  group: "GitHub",
  capture: "github-diff",
  viewport: "1024x680",
  check: "When the code lane cannot hold about 80 terminal cells, logical lines stay intact and horizontal scrolling remains available.",
});
add({
  slug: "github-diff-binary",
  title: "GitHub diff · no textual patch",
  group: "GitHub",
  capture: "github-diff-binary",
  check: "Binary and oversized patches explain why no text is available without collapsing either surface.",
});
add({
  slug: "github-diff-loading",
  title: "GitHub diff · loading",
  group: "GitHub",
  capture: "github-diff-loading",
  check: "Loading preserves the full viewer and file navigation layout.",
});
add({
  slug: "github-diff-error",
  title: "GitHub diff · error",
  group: "GitHub",
  capture: "github-diff-error",
  check: "Diff errors remain scoped to the viewer and leave file navigation available.",
});
add({
  slug: "github-blocked",
  title: "GitHub · checks failing",
  group: "GitHub",
  capture: "github-blocked",
  check: "Two failed checks and a BLOCKED merge state must read as blocked, not merely noted.",
});
add({
  slug: "github-draft-pr",
  title: "GitHub · draft PR",
  group: "GitHub",
  capture: "github-draft-pr",
  check: "Draft badge plus pending checks.",
});
add({
  slug: "github-merge-confirmation",
  title: "GitHub · merge confirmation",
  group: "GitHub",
  capture: "github-merge-confirmation",
  check: "Confirmation step before an irreversible merge.",
});
add({
  slug: "github-merging",
  title: "GitHub · merging",
  group: "GitHub",
  capture: "github-merging",
  check: "In-flight merge: controls must be disabled, not just visually busy.",
});
add({
  slug: "github-no-pr",
  title: "GitHub · no open pull requests",
  group: "GitHub",
  capture: "github-no-pr",
  check: "The PR tab explains that the repository has no open pull requests.",
});
add({
  slug: "github-auth",
  title: "GitHub · needs sign-in",
  group: "GitHub",
  capture: "github-auth",
  check: "Unauthenticated panel with a single clear next action.",
});
add({
  slug: "github-auth-collapsed",
  title: "GitHub · sign-in, rail collapsed",
  group: "GitHub",
  capture: "github-auth-collapsed",
  check: "Same panel with the rail collapsed.",
});
add({
  slug: "github-loading",
  title: "Local changes · loading",
  group: "GitHub",
  capture: "github-loading",
  check: "The Local tab loader reserves the panel shape while actions remain unavailable.",
});
add({
  slug: "github-pull-requests-loading",
  title: "Pull requests · loading",
  group: "GitHub",
  capture: "github-pull-requests-loading",
  check: "Opening the PR tab shows the nine-dot loader without exposing stale list rows or refresh actions.",
});
add({
  slug: "github-refreshing",
  title: "Local changes · refreshing",
  group: "GitHub",
  capture: "github-refreshing",
  check: "Refresh replaces stale local files with the nine-dot loader; repository identity and Close remain while Refresh and file actions disappear.",
});
add({
  slug: "github-error",
  title: "Local changes · Git error",
  group: "GitHub",
  capture: "github-error",
  check: "A local Git error stays scoped to the Local tab with a direct retry action.",
});
add({
  slug: "github-unavailable",
  title: "GitHub · CLI unavailable",
  group: "GitHub",
  capture: "github-unavailable",
  check: "gh not installed — the panel should explain the prerequisite.",
});

// ── Terminal rendering ──────────────────────────────────────────────────────
add({
  slug: "terminal-glyphs",
  title: "Box-drawing & block glyphs",
  group: "Terminal rendering",
  capture: "terminal-glyphs",
  check:
    "Rules, blocks, rounded and heavy borders must be pixel-continuous with no gaps at cell seams.",
});
add({
  slug: "terminal-palette",
  title: "ANSI palette & attributes",
  group: "Terminal rendering",
  capture: "terminal-palette",
  check: "Both ANSI ramps, bold/dim/italic/underline/reverse, and truecolor in the default theme.",
});

// ── Settings ────────────────────────────────────────────────────────────────
add({
  slug: "settings",
  title: "Settings · Preferences",
  group: "Settings",
  capture: "settings",
  check: "Section rhythm, control alignment, and the live terminal preview.",
});
add({
  slug: "settings-compact",
  title: "Settings · 820×560",
  group: "Settings",
  capture: "settings",
  viewport: "820x560",
  check: "Settings at compact width — controls must not crowd their labels.",
});
add({
  slug: "settings-minimum",
  title: "Settings · 720×480",
  group: "Settings",
  capture: "settings",
  viewport: "720x480",
  check: "Settings at the minimum supported window.",
});
add({
  slug: "settings-wide",
  title: "Settings · 1920×1080",
  group: "Settings",
  capture: "settings",
  viewport: "1920x1080",
  check: "Settings must not stretch its measure to the full desktop width.",
});
add({
  slug: "theme-gallery",
  title: "Theme gallery",
  group: "Settings",
  capture: "theme-gallery",
  check: "Every preset as a live preview — grid rhythm and caption legibility on light presets.",
});
add({
  slug: "theme-gallery-compact",
  title: "Theme gallery · 820×560",
  group: "Settings",
  capture: "theme-gallery",
  viewport: "820x560",
  check: "Preview grid reflow at compact width.",
});
add({
  slug: "theme-gallery-wide",
  title: "Theme gallery · 1920×1080",
  group: "Settings",
  capture: "theme-gallery",
  viewport: "1920x1080",
  check: "Preview grid at desktop width.",
});

// ── Appearance ──────────────────────────────────────────────────────────────
for (const [slug, title, appearance] of [
  ["light", "Light", "light"],
  ["dark", "Dark", "dark"],
]) {
  for (const [surface, surfaceTitle, capture] of [
    ["workspace", "Workspace", "-"],
    ["settings", "Settings", "settings"],
    ["palette", "Command palette", "palette"],
    ["fleet-agents", "Fleet · Agents", "fleet-agents"],
    ["github-panel", "GitHub panel", "github-panel"],
    ["github-diff", "GitHub diff", "github-diff"],
    ["worktree-manager", "Worktrees", "worktree-manager"],
    ["theme-gallery", "Theme gallery", "theme-gallery"],
    ["needs-input", "Agent needs input", "needs-input"],
    ["session-picker", "Session picker", "session-picker"],
    ["terminal", "ANSI palette", "terminal-palette"],
  ]) {
    add({
      slug: `appearance-${slug}-${surface}`,
      title: `${title} · ${surfaceTitle}`,
      group: `Appearance · ${title}`,
      capture,
      settings: { appearance },
      check: `${surfaceTitle} with the ${title.toLowerCase()} chrome palette — contrast of secondary text and borders.`,
    });
  }
}

// ── Terminal themes ─────────────────────────────────────────────────────────
for (const [id, name, isLight] of THEMES) {
  add({
    slug: `theme-${id}`,
    title: name,
    group: "Terminal themes",
    capture: "terminal-palette",
    settings: { terminal_theme: id, appearance: isLight ? "light" : "dark" },
    check: `${name}${isLight ? " (light preset)" : ""} — all 16 ANSI colours distinguishable, and the chrome/terminal seam.`,
  });
}

// ── Typography ──────────────────────────────────────────────────────────────
for (const [size, note] of [
  [12, "smallest"],
  [14, "reference"],
  [20, "largest"],
]) {
  add({
    slug: `ui-font-size-${size}`,
    title: `UI type ${size}pt (${note})`,
    group: "Typography · UI",
    capture: "-",
    settings: { ui_font_size: size },
    check: "Rail rows have fixed anatomy — secondary copy must truncate sooner rather than grow the row.",
  });
}
add({
  slug: "ui-font-size-20-settings",
  title: "UI type 20pt · Settings",
  group: "Typography · UI",
  capture: "settings",
  settings: { ui_font_size: 20 },
  check: "Largest interface type in the densest surface.",
});
add({
  slug: "ui-font-size-12-fleet-agents",
  title: "UI type 12pt · Fleet Agents",
  group: "Typography · UI",
  capture: "fleet-agents",
  settings: { ui_font_size: 12 },
  check: "Smallest interface type: agent names get a larger character budget.",
});
// The app refuses an interface weight the chosen family does not ship, so the
// variations here are the weights the resolved system sans genuinely installs.
// Asking for more only captures the "weight reset" warning.
for (const [weight, label] of [
  ["normal", "Regular"],
  ["semibold", "Semibold"],
]) {
  add({
    slug: `ui-font-weight-${weight}`,
    title: `UI weight ${label}`,
    group: "Typography · UI",
    capture: "-",
    settings: { ui_font_weight: weight },
    check: "Weight must not change row heights or push labels out of alignment.",
  });
}
for (const family of ["DejaVu Sans", "Ubuntu Sans", "Liberation Sans", "FreeSans"]) {
  add({
    slug: `ui-font-${family.toLowerCase().replaceAll(" ", "-")}`,
    title: `UI font · ${family}`,
    group: "Typography · UI",
    capture: "-",
    settings: { ui_font: family },
    check: "A substituted interface family must not break measured layout or clip glyph tails.",
  });
}

for (const [size, note] of [
  [10, "smallest"],
  [14, "default"],
  [20, "large"],
  [28, "largest"],
]) {
  add({
    slug: `terminal-font-size-${size}`,
    title: `Terminal type ${size}pt (${note})`,
    group: "Typography · Terminal",
    capture: "terminal-palette",
    settings: { terminal_font_size: size },
    check: "Cell advance is measured from the face — the grid must stay flush with the pane edges.",
  });
}
for (const height of [1.0, 1.15, 1.6]) {
  add({
    slug: `terminal-line-height-${String(height).replace(".", "-")}`,
    title: `Terminal line height ${height}`,
    group: "Typography · Terminal",
    capture: "terminal-palette",
    settings: { terminal_line_height: height },
    check: "Block glyphs must stay vertically continuous across rows at every line height.",
  });
}
for (const family of [
  "FiraCode Nerd Font Mono",
  "Liberation Mono",
  "Ubuntu Mono",
  "DejaVu Sans Mono",
  "Ubuntu Sans Mono",
]) {
  add({
    slug: `terminal-font-${family.toLowerCase().replaceAll(" ", "-")}`,
    title: `Terminal font · ${family}`,
    group: "Typography · Terminal",
    capture: "terminal-palette",
    settings: { terminal_font: family },
    check: "Advance-ratio measurement per face — runs must not drift right or clip their tails.",
  });
}
add({
  slug: "terminal-font-firacode-glyphs",
  title: "FiraCode · box drawing",
  group: "Typography · Terminal",
  capture: "terminal-glyphs",
  settings: { terminal_font: "FiraCode Nerd Font Mono" },
  check: "Box-drawing continuity with a face whose own box glyphs differ from the fallback.",
});
for (const [weight, label] of [
  ["light", "Light"],
  ["medium", "Medium"],
  ["bold", "Bold"],
  ["semibold", "Semibold"],
]) {
  add({
    slug: `terminal-font-weight-${weight}`,
    title: `Terminal weight ${label}`,
    group: "Typography · Terminal",
    capture: "terminal-palette",
    // System monospace installs Regular and Bold only; the app resets any
    // weight the family does not ship, so ask a family that has all four.
    settings: { terminal_font: "FiraCode Nerd Font Mono", terminal_font_weight: weight },
    check: "Bold-within-bold: the attribute row's bold sample must still differ from regular.",
  });
}

// ── Misc configuration ──────────────────────────────────────────────────────
add({
  slug: "status-bar-visible",
  title: "Status bar shown",
  group: "Configuration",
  capture: "-",
  settings: { show_status_bar: true },
  check: "Optional status bar: content and the height it takes from the panes.",
});
add({
  slug: "status-bar-visible-compact",
  title: "Status bar shown · 820×560",
  group: "Configuration",
  capture: "-",
  viewport: "820x560",
  settings: { show_status_bar: true },
  check: "Status bar at compact height, where vertical space is scarcest.",
});
add({
  slug: "fleet-view-agents-persisted",
  title: "Agents view persisted in settings",
  group: "Configuration",
  capture: "-",
  settings: { fleet_view: "agents" },
  check: "The saved fleet view must be honoured on boot without a staged override.",
});
add({
  slug: "fleet-view-repos-persisted",
  title: "Repos view persisted in settings",
  group: "Configuration",
  capture: "-",
  settings: { fleet_view: "repos" },
  check: "Repos on boot with no repository metadata yet — the grouping must degrade gracefully.",
});

add({
  slug: "github-long-login",
  title: "GitHub · long login in rail footer",
  group: "GitHub",
  capture: "github-long-login",
  check: "A 39-character login must ellipsize instead of running under the signal dot.",
});
add({
  slug: "hook-repair",
  title: "Settings · hooks need repair",
  group: "Settings",
  capture: "hook-repair",
  check: "A hook whose muxtrixctl was removed must say why it cannot work, not read as installed.",
});

export default cases;
