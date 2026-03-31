load("@prelude//platforms:defs.bzl", "host_configuration")

def _executor_config():
    """Return a CommandExecutorConfig with RE enabled only when configured."""
    if read_config("buck2_re_client", "engine_address"):
        return CommandExecutorConfig(
            local_enabled = True,
            remote_enabled = True,
            use_limited_hybrid = True,
            remote_execution_properties = {
                "OSFamily": "Linux",
                "container-image": "docker://gcr.io/flame-public/rbe-ubuntu20-04:latest",
            },
            remote_execution_use_case = "buck2-default",
            remote_output_paths = "output_paths",
        )
    else:
        return CommandExecutorConfig(
            local_enabled = True,
            remote_enabled = False,
        )

def _buildbuddy_platforms(ctx):
    constraints = dict()
    constraints.update(ctx.attrs.cpu_configuration[ConfigurationInfo].constraints)
    constraints.update(ctx.attrs.os_configuration[ConfigurationInfo].constraints)
    configuration = ConfigurationInfo(constraints = constraints, values = {})

    platform = ExecutionPlatformInfo(
        label = ctx.label.raw_target(),
        configuration = configuration,
        executor_config = _executor_config(),
    )

    return [
        DefaultInfo(),
        PlatformInfo(label = str(ctx.label.raw_target()), configuration = configuration),
        ExecutionPlatformRegistrationInfo(platforms = [platform]),
    ]

buildbuddy_platforms = rule(
    impl = _buildbuddy_platforms,
    attrs = {
        "cpu_configuration": attrs.dep(providers = [ConfigurationInfo]),
        "os_configuration": attrs.dep(providers = [ConfigurationInfo]),
    },
)
