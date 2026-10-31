# Ribbon Plugin System (`ribbon_plugins`)
Provides plugins, ordered plugin groups, dependency checks, and deferred finish hooks.
Plugins configure an `ribbon_app::App` synchronously when added.
Allowed workspace dependency: `ribbon_app`.
It does not provide dynamic loading or parallel plugin initialization.
