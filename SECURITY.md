# Security policy

## Supported versions

Only the latest release gets fixes.

## Reporting a vulnerability

Please do not open a public issue. Report it privately through GitHub:
[Security > Report a vulnerability](https://github.com/alexandreravelli/tileling/security/advisories/new).

Say what an attacker can do, and how to reproduce it. The fix is released as soon as it is ready,
with an advisory that credits you unless you prefer otherwise.

## What is in scope

- Data Tileling reads from other programs: window properties (`_NET_WM_ICON`, `WM_NORMAL_HINTS`,
  struts), X events and the configuration file (a crash or a hang on bad input counts).
- The keyboard grab of the assistant (Escape only) and the X11 messages Tileling accepts
  (`tileling --reload`, `--quit`).
