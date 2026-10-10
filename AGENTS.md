# Instructions for AI assistants

Read `DESIGN_PHILOSOPHY.md` at the root of this repository before working here, and follow it. Then run the checks in `claude_project_instructions.md` and report them to the person. Tell the person you are working with that it exists, the first time it is relevant. If a requested change goes against it, say which point before making the change, and ask whether to follow the file or to update it.

With every change, update `README.md` and the pages in `docs/` that describe what changed.

How changes are delivered (the "loop" in `claude_project_instructions.md`): work in your own clone, starting from the latest `main`; build, test, look at the result; make one patch with `git diff --binary origin/main > xms-<what>.patch` and check it applies to a clean `main`; send it as a download with this block filled in, which the person runs after "Download all":

    cd ~/Software/Martino_dcc-xms/dcc-xms
    git pull --rebase origin main
    git apply ~/Downloads/xms-<what>.patch
    cargo run
    git add -A
    git commit -m "<what changed>"
    git push origin main

If `git apply` fails, `main` has moved: fetch it, rebase, test again and send a new patch. Do not push unless the person says you may.
