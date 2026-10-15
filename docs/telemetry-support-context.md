# Deployment labels (formerly "telemetry support context")

Freenpipe has no telemetry and no crash reporting: nothing about a run is sent
to any analytics or error-tracking service, whatever the settings say.

The environment variables below are still read, for compatibility with
deployments that already set them. Their only effect is to label the local
crash record: when the standalone engine (`screenpipe record`) panics, the
entry written to `last-panic.log` in its data directory ends with the labels
that were set, so an operator who collects that file can tell which deployment
produced it.

```bash
export SCREENPIPE_SUPPORT_ID="spcust_acme_123"
export SCREENPIPE_CUSTOMER_ID="acme"
export SCREENPIPE_DEPLOYMENT_ID="prod-laptop-fleet-01"
export SCREENPIPE_EMBEDDER="acme-agent"
export SCREENPIPE_EMBEDDER_VERSION="2026.6.4"

screenpipe record
```

## Environment variables

| Variable | Written to the crash record as |
| --- | --- |
| `SCREENPIPE_SUPPORT_ID` | `screenpipe_support_id` |
| `SCREENPIPE_CUSTOMER_ID` | `screenpipe_customer_id` |
| `SCREENPIPE_DEPLOYMENT_ID` | `screenpipe_deployment_id` |
| `SCREENPIPE_EMBEDDER` | `screenpipe_embedder` |
| `SCREENPIPE_EMBEDDER_VERSION` | `screenpipe_embedder_version` |

Accepted aliases (the first one set wins):

| Canonical variable | Accepted aliases |
| --- | --- |
| `SCREENPIPE_SUPPORT_ID` | `SCREENPIPE_TELEMETRY_ID` |
| `SCREENPIPE_CUSTOMER_ID` | `SCREENPIPE_ORG_ID`, `SCREENPIPE_TELEMETRY_CUSTOMER_ID` |
| `SCREENPIPE_DEPLOYMENT_ID` | `SCREENPIPE_TELEMETRY_DEPLOYMENT_ID` |
| `SCREENPIPE_EMBEDDER` | `SCREENPIPE_HOST_APP`, `SCREENPIPE_TELEMETRY_HOST_APP` |
| `SCREENPIPE_EMBEDDER_VERSION` | `SCREENPIPE_HOST_VERSION`, `SCREENPIPE_TELEMETRY_HOST_VERSION` |

`SCREENPIPE_ANALYTICS_ID` (falling back to `SCREENPIPE_SUPPORT_ID`) is used
only as a local salt for hashed identifiers in debug logs. The desktop app sets
it to its per-install id when it starts the engine.

`SCREENPIPE_DISTRIBUTION` / `SCREENPIPE_DIST` used to tag analytics events with
the install channel; they are no longer read.

## Privacy

Use opaque ids, not emails or names: the labels end up in a plain-text log file
on the machine. `--disable-telemetry` and the `analyticsEnabled` setting are
still accepted for compatibility and change nothing.
