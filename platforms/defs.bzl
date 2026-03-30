def _buildbuddy_platforms(ctx):
    configuration = ConfigurationInfo(
        constraints = {},
        values = {},
    )

    image = "docker://gcr.io/flame-public/rbe-ubuntu20-04:latest"
    platform = ExecutionPlatformInfo(
        label = ctx.label.raw_target(),
        configuration = configuration,
        executor_config = CommandExecutorConfig(
            local_enabled = True,
            remote_enabled = True,
            use_limited_hybrid = True,
            remote_execution_properties = {
                "OSFamily": "Linux",
                "container-image": image,
                "dockerNetwork": "bridge",
            },
            remote_execution_use_case = "buck2-default",
            remote_output_paths = "output_paths",
        ),
    )

    return [
        DefaultInfo(),
        PlatformInfo(label = str(ctx.label.raw_target()), configuration = configuration),
        ExecutionPlatformRegistrationInfo(platforms = [platform]),
    ]

buildbuddy_platforms = rule(attrs = {}, impl = _buildbuddy_platforms)
