# Working on XMS | Imago with an AI assistant

How Martino and Simon work on this repository with Claude, and what an assistant should check so that anyone pulling the repository can work the same way.

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

- **Every change on top of the latest `main`.** Before starting, fetch `main`; a change is made against it.
- **Delivered as a patch, pushed by the person.** Unless the person says otherwise, the assistant hands over a `.patch` file (made with `git diff --binary`, so images and example data come with it) and the commands to apply it:

      cd <the repository>
      git pull --rebase origin main
      git apply ~/Downloads/<name>.patch
      cargo run
      git add -A
      git commit -m "<what changed>"
      git push origin main

  The person runs every git and GitHub command. Check that the patch applies to a clean copy of `main` before handing it over.
- **Tests with every feature.** New behaviour comes with tests; `cargo test` passes before a change is handed over.
- **Documentation with every change.** `README.md` and the pages in `docs/` that describe what changed are updated in the same change. The website (`site/`) only when something on it is wrong.
- **A screenshot when something visible changed**, in `docs/`, taken from the running program.
- **Say what was tested and what was not.** A claim in the docs is measured where it can be.
- **Files under 100 MB.** GitHub refuses larger ones; example data goes in the program's compact formats (`.xmsclip`, `.xmsmesh`).
- **Credit.** XMS was created by Martino Madeddu. Additional development by Simon Legrand.

## Flag what does not fit

If any of this cannot be done in your setup (no workspace, no GitHub access, a different workflow the person asks for), tell the person which point does not hold and what changes because of it, before starting work.
