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
