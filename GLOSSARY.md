# Handover

Handover runs several AI coding CLIs side by side on one project and passes the work from one to another.

## Language

**Project**:
A folder on the user's machine that Handover manages, with its own settings and terminals.
_Avoid_: Workspace, repo

**Instance**:
One terminal belonging to a Project, running in a single Sandbox Mode for its whole life.
_Avoid_: Tab, container, shell

**Terminal Session**:
The live connection between an Instance and the window showing it: what the user types goes in, and what the terminal prints comes out.
_Avoid_: Bridge, socket, PTY bridge

**Sandbox Mode**:
Where an Instance runs: `native` on the user's machine, or `docker` in an isolated container.
_Avoid_: Runtime, environment

**Handoff**:
Passing a Project's in-progress work from one Instance's AI to another's, so the second can continue where the first stopped.
_Avoid_: Handover (that is the product name), transfer

**Handoff Method**:
How a Handoff carries the work: `summary` (the source AI writes a handoff file) or `git` (the work is committed).

**Handoff Template**:
A preset that tailors the summary a source AI is asked to write to a kind of project (`generic`, `nextjs`, `rust-cli`).

**Broadcast**:
Sending one prompt to every Instance of a Project at once.

**Governor**:
The watchdog that pauses idle Projects' Docker Instances when the machine runs low on memory.
