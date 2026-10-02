# CLI workflow upgrade

Research and implementation: 2026-10-02. This is a comparison of documented
interaction patterns, not a performance benchmark or a claim that every CLI
has been tested hands-on. The existing inline transcript remains the foundation. Official Pi, OpenCode
and OMP screenshot assets were also inspected for spacing and hierarchy.

## What makes a terminal agent feel smooth

| Reference | Useful pattern | Pactrail decision |
| --- | --- | --- |
| [Codex CLI customization](https://learn.chatgpt.com/docs/cli-customization) | External editor returns a draft before sending; readable code/diffs and terminal-specific controls | Preserve Ctrl+G and verbatim code wrapping. Make draft preservation explicit and restart-safe. |
| [OpenCode TUI](https://opencode.ai/docs/tui/) and [keybindings](https://opencode.ai/docs/keybinds/) | Command palette, session navigation, file completion, separate presentation settings | Rank command matches; search run goals; complete task paths; keep review actions explicit. |
| [Pi terminal usage](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/usage.md) | Editor, transcript, contextual footer, path completion and saved sessions | Keep permissions visible in the composer header, protect pasted tasks, save workspace drafts and previous tasks. |
| [OMP](https://github.com/can1357/oh-my-pi) and [CLI reference](https://omp.sh/docs/cli) | Practical terminal-first interactions with structured tool presentation and selectable actions | Retain typed live events and bounded detail. Selection must not execute model work or apply files. |
| [Aider commands](https://aider.chat/docs/usage/commands.html) | Discoverable slash commands and distinct local operations | Keep ordinary prose untouched by completion; local composer operations do not call a model. |

The design conclusion: smoothness comes from preserving user intent, providing
short paths to common actions, and making the current state legible. Decoration
alone cannot deliver that. These patterns do not justify inventing reasoning,
output, token measurements, or successful verification.

## Implemented scope

1. **Composer:** explicitly enable bracketed paste. Ctrl+J inserts a line; Enter
   submits. Ctrl+G round-trips through the user's editor. Ctrl+S saves exactly the
   current text, leaving it editable. It is not per-keystroke autosave.
2. **Drafts:** `/draft` restores without sending; `/draft clear` removes the saved
   draft. Startup advertises a saved draft. Ctrl+C clears only the current editor.
   Saved drafts remain until explicitly removed or replaced.
3. **Retry:** `/retry` restores the previous submitted task after restart, scoped
   to the canonical workspace. It does not resume a checkpoint or automatically
   resubmit, and image attachments are not restored.
4. **Discovery:** command prefixes rank before fuzzy matches. Explicit run argument
   completion searches both IDs and goal descriptions; model completion is fuzzy.
   Ctrl+O puts `/focus ` in the editor; type a goal fragment and Tab. A nonempty
   task is saved first; a save failure aborts navigation and preserves the editor.
5. **Files:** `/task <prefix>` completes regular files/directories in one directory,
   quotes whitespace safely and preserves Unicode. It does not attach contents
   automatically, recursively index a repository, or follow symlink entries.
6. **Presentation:** a thin composer rail separates editable input from the
   durable transcript. Multiline input has a consistent vertical guide. Run history combines ID, state and changed-file count into a
   compact identity row. Outcome and goal remain visible. Narrow prompts retain
   command permissions, with model details in the startup header and `/status`.

## Persistence, authority and compatibility

Composer text lives in the user's config directory, under `composer/`, outside
the candidate/source workspace. Names include a BLAKE3 digest of the canonical
workspace path; paths are not used directly as filenames. Draft and last-task
files are separate. Writes use synced private temporary files and atomic
replacement. On Unix the directory is 0700 and files 0600. Reads reject symlinks,
invalid UTF-8 and more than 64 KiB. Directory symlinks are rejected on writes.
These files are local plaintext: do not put credentials in a task. They are not
engine evidence or cross-workspace knowledge. Last writer wins if two CLI
sessions explicitly save the same workspace draft.

No runtime dependency, engine/event/receipt schema, provider protocol, tool
permission or automation subcommand changes. Apply and Discard retain typed
confirmation with default cancel. Run selection only changes focus. Completion
never rewrites a prose task. The full verified trace and terminal scrollback
remain available; UI content is sanitized before display.

## Verification

Unit tests cover persistence/restart/isolation, atomic replacement, private file
modes, invalid/oversized text, symlinks, Unicode/quoted paths, matching and spans.
`devtools/check_cli_experience.py` runs the real binary in disposable PTYs with a
local model fixture: paste without accidental dispatch, saved draft restore and
remove, restart retry, fuzzy commands, run goal search, task completion,
multiline task, external editor, pager return, and default-cancel/explicit
Apply/Discard. Widths 32/40/60/80/120, NO_COLOR and TERM=dumb are checked.
The suite writes terminal captures to `/tmp/pactrail-cli-qa`; no credentials or
real project tasks are used.

## Next separate upgrade

Plain `continue` needs durable task selection and engine-bound recovery semantics,
not a string alias for sending another new task. Handle ambiguity, terminal
results, unsafe checkpoints, revision-bound context and permissions explicitly.
Implemented as the separate [persistent task continuation](persistent-task-continuation.md) upgrade after the composer/navigation commit.

## Visual refinement

The transcript remains inline and selectable. A cyan wordmark anchors startup;
neutral bold headings distinguish sections without competing with warnings.
Model activity retains its magenta accent, and permissions stay visible above
the composer. Terminal foreground colors are used for prose and code, so the
terminal owns the light or dark background. No background blocks or truecolor
assumptions are introduced. `NO_COLOR` and `TERM=dumb` remain supported.

Startup shortcuts have three balanced rows. Below 60 columns, metadata labels
stack above their values rather than consuming half the reading width. Live
activity puts the category before its timestamp; receipt output has its own
section boundary, and reports have a consistent two-cell reading inset.

The real PTY suite captures colored startup, answered, and review states at 40 and 100
columns, alongside plain 32–120-column flows. Captures are local development
artifacts in `/tmp/pactrail-cli-qa`, not application dependencies.
