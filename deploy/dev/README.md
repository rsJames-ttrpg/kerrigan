# Command Reference

Claude creds mount and mod
```sh
cp ~/.claude/.credentials.json /tmp/claude-creds.json && chmod 644 /tmp/claude-creds.json
```

Run the docker file

```sh
docker run -it --rm -p 3100:3100 \
    -v kerrigan-live:/data \
    -v /tmp/claude-creds.json:/home/kerrigan/.claude/.credentials.json:ro \
    -v $(pwd)/deploy/dev/hatchery.local.toml:/opt/kerrigan/config/hatchery.toml:ro \
    -e SIGNAL_API_TOKEN=$SIGNAL_API_KEY \
    kerrigan
```

## Repo Credentials

Associate a GitHub PAT with a repo so drones get it automatically (no more `--set secrets.github_pat=...` on every submit).

### Via CLI

```sh
# org-wide wildcard — covers all repos under rsJames-ttrpg
echo "ghp_your_pat_here" | kerrigan creds add \
    --pattern "github.com/rsJames-ttrpg/*" \
    --type github_pat

# or for a specific repo
echo "ghp_your_pat_here" | kerrigan creds add \
    --pattern "github.com/rsJames-ttrpg/kerrigan" \
    --type github_pat

# check what's configured
kerrigan creds list
```

### Via overseer.toml (deploy-time)

Add to `overseer.toml` (or a mounted config):

```toml
[[credentials]]
pattern = "github.com/rsJames-ttrpg/*"
credential_type = "github_pat"
secret_env = "GITHUB_PAT"
```

Then pass the env var to the container:

```sh
docker run -it --rm -p 3100:3100 \
    -v kerrigan-live:/data \
    -v /tmp/claude-creds.json:/home/kerrigan/.claude/.credentials.json:ro \
    -v $(pwd)/deploy/dev/hatchery.local.toml:/opt/kerrigan/config/hatchery.toml:ro \
    -e SIGNAL_API_TOKEN=$SIGNAL_API_KEY \
    -e GITHUB_PAT=ghp_your_pat_here \
    kerrigan
```

Credentials are upserted on startup, so restarts are idempotent. The most specific pattern wins — an exact repo match takes precedence over an org wildcard.
