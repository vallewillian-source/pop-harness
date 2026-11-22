# Security policy

## Versions that receive fixes

POP Harness is in the `0.x` series. Until `1.0`, only the latest published version and `main` receive
security fixes.

| Version | Receives security fixes |
|---|---|
| `main` and the latest published `0.x` | yes |
| earlier `0.x` versions | no |

## How to report a vulnerability

**Do not open a public issue, pull request or discussion** to talk about a security flaw.

Use GitHub's private reporting:

1. Open the **Security** tab of the repository.
2. Click **Report a vulnerability**.
3. Describe the problem.

Include, if you can:

- what happens and what the impact is (for example: one org reads another org's data, a secret leak, code
  execution through a flow script, denial of service);
- steps to reproduce, or a proof of concept;
- the affected version or commit, the operating system and the relevant configuration;
- a suggested fix, if you have one.

Do not include real customer data, tokens or keys. If the proof of concept needs them, use test values.

## What to expect

- We acknowledge receipt within **5 business days**.
- We assess, reproduce and reply with what we concluded and what we will do.
- We follow coordinated disclosure: we work on the fix with you and publish the details within **90 days**
  of the report, or earlier when the fix ships. If we need more time, we say why and agree on a new date.
- We credit the reporter, if you want.

## What we consider a security flaw in this project

In particular, in order of severity:

1. **Broken isolation between orgs**: an org, org token or conversation reaching another org's data.
2. **Secret leak**: a key, token or integration secret appearing in a log, API response, error message,
   backup or file.
3. **Privilege escalation**: an org token performing a global action; a call without a token being accepted.
4. **Execution or abuse from untrusted input**: a flow script, a received file (PDF and Office), a customer
   message treated as an instruction, a reply from an external plugin.
5. **Denial of service** that a customer or flow author can cause to other orgs.

Ordinary bugs without security impact (a badly documented configuration value, a dependency advisory with
no exploit path) can go in a normal public issue.

## Good practices for whoever runs an instance

- Keep `POPH_CHAVE_CIFRA` in a secrets manager. Whoever loses it loses access to the encrypted secrets.
- The Console and the admin API are not designed for the open internet: put a reverse proxy with TLS in
  front and restrict who can reach `/admin/v1`.
- Use an org token, not the global one, for everything that is not administering the server.
- Run `poph doctor` after every configuration change.
- Keep `cargo audit` and `cargo deny` in your pipeline if you build from source.
