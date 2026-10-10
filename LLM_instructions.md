# Instructions for AI assistants

The one instruction file for any AI assistant (Claude, ChatGPT, Copilot, Cursor or other) working on this repository: what to check before starting, and how Martino and Simon work on it, so that anyone pulling the repository can work the same way.

If you are an AI assistant: read this file and `DESIGN_PHILOSOPHY.md` before working here, run the checks below and report them to the person, and follow both. With every change, update `README.md` and the pages in `docs/` that describe what changed.

## For AI assistants: run these checks first

At the start of a session in this repository, run the checks below and report each one to the person as passed or failed, with the fix when it failed. Do not skip a check because it is probably fine. When something cannot be checked from where you run, say so instead of guessing.

1. **Design philosophy.** Read `DESIGN_PHILOSOPHY.md` and tell the person it exists. Follow it; when a request goes against it, say which point and ask whether to follow the file or update it.

2. **A workspace that runs commands.** Check that you can run shell commands and keep files between turns. Without it you can only advise, not build, test or produce patches: tell the person, and that their Claude plan or app has to offer it.

3. **The program builds and its tests pass.** Clone the repository, install what the README's "Build from source" lists (on Ubuntu or Debian: `build-essential pkg-config libasound2-dev libudev-dev libx11-dev libxkbcommon-x11-0`, plus `libwayland-dev libxkbcommon-dev` for a full build), then run `cargo test`. The first build takes a long time. Report how many tests passed. If the examples folder is not found, set `XMS_EXAMPLES` to the repository's `examples` folder.

4. **GitHub access.** Check that you can read `MartinoMadeddu/xms-imago`, then whether you can push to it.
   - Reading needs nothing for a public repository.
   - Pushing needs the Claude GitHub App installed on the account that owns the repository, with access to `xms-imago` ([install it here](https://github.com/apps/claude/installations/select_target)). Only the owner (Martino) can install it on that account; a collaborator's own installation does not reach it. If it is installed but not linked, the person can re-link GitHub from [Claude's connector settings](https://claude.ai/customize/connectors?auth_start=github&auth_start_force=1).
   - If pushing is not possible, or the person prefers it, use the patch workflow below. Do not push unless the person has said you may.

5. **Settings worth switching on in the Claude app.** Ask the person to check these in Settings, and say what each one is for:
   - **Web search**: research such as skeleton naming conventions, file formats and joint ranges was done with it.
   - **Generate memory from chats** and **Search and reference past chats**: standing rules (how changes are delivered, who pushes, documentation with every change) carry over between conversations instead of being repeated.
   - Optional: the Claude desktop app linked to the person's computer, so files on it can be reached directly instead of attached.

6. **Report.** End the checks with one list: what passed, what failed, and what the person has to do.

## How we work

### The loop

This is how a change goes from a request in a Claude chat to `main`, with the person running only one block of commands:

1. **The person asks for a change** in a Claude chat (claude.ai, a Project or the desktop app), in plain words.
2. **The assistant works in its own copy.** In its workspace it keeps a clone of `MartinoMadeddu/xms-imago` and, before every change, fetches `main` and starts from it, so the change is made against what is really there.
   - It reads what Martino pushed since the last change (`git log --author=Martino`, his diffs and his docs), and builds on it: his models and functions are used and extended, not replaced. A change that would rewrite something of his is named and asked about first.
   - It adds to `DESIGN_PHILOSOPHY.md` the decisions his new code shows, in the same patch, with a line in its log.

   It builds, runs `cargo test`, runs the program to look at the result (a screenshot under a virtual display: `xvfb-run` with `WGPU_BACKEND=gl LIBGL_ALWAYS_SOFTWARE=1`), and updates `README.md` and `docs/`.
3. **The assistant makes one patch** of everything since `origin/main`: `git diff --binary origin/main > xms-<what>-<suffix>.patch`. The suffix is new every time (date and time, `DDMMYYYY-HHMM`): a file sent again under a name already used replaces the earlier one in the chat instead of arriving as a new download, and "Download all" does not appear. `--binary` carries images, example files and solved data. It then checks the patch on a clean copy of `main` (`git worktree add` on `origin/main`, `git apply`), so it is known to apply before it is handed over.
4. **The assistant sends the files**: the patch, and any screenshot worth looking at, as downloads in the chat, with the block of commands below filled in (patch name and commit message). It says what changed, what was tested and what was not.
5. **The person clicks "Download all"**, so the files land in `~/Downloads`, and pastes the block into a terminal:

       cd ~/Software/Martino_dcc-xms/dcc-xms
       git pull --rebase origin main
       git apply ~/Downloads/xms-<what>-<suffix>.patch
       cargo run
       git add -A
       git commit -m "<what changed>"
       git push origin main

   `git pull --rebase` brings in what others pushed; `git apply` adds the change; `cargo run` lets the person look before committing; the last three lines commit and push it. (The path in the first line is Simon's clone; use your own.)
6. **When `git apply` fails**, someone pushed to `main` in between and the patch no longer fits. The person pastes the terminal output into the chat; the assistant fetches `main` again, rebases its work onto it (keeping the other person's version where both changed the same lines of documentation, and saying so), runs the tests again and sends a new patch. Nothing is lost: a failed `git apply` changes nothing, and the commands after it have nothing to commit.
7. **The next change starts from the last patch.** Until the person says a patch is pushed, the assistant keeps its work committed locally (never pushed) and makes the next patch on top of it, saying in which order to apply them; or folds them into one.

What this needs:

- On the person's computer: a clone of the repository, `git` signed in to GitHub with push rights to it (as owner or collaborator), and Rust with the libraries in the README, for `cargo run`.
- For the assistant: a workspace that runs commands and reaches GitHub (check 2 and 4 above). Pushing from the workspace is not needed; the person pushes.

### Conventions

- **Tests with every feature.** New behaviour comes with tests; `cargo test` passes before a change is handed over.
- **Documentation with every change.** `README.md` and the pages in `docs/` that describe what changed are updated in the same change. The website (`site/`) only when something on it is wrong.
- **A screenshot when something visible changed**, in `docs/`, taken from the running program.
- **Say what was tested and what was not.** A claim in the docs is measured where it can be.
- **Files under 100 MB.** GitHub refuses larger ones; example data goes in the program's compact formats (`.xmsclip`, `.xmsmesh`).
- **Credit.** XMS was created by Martino Madeddu. Additional development by Simon Legrand.

## Flag what does not fit

If any of this cannot be done in your setup (no workspace, no GitHub access, a different workflow the person asks for), tell the person which point does not hold and what changes because of it, before starting work.
