# engine_plugins
Provides plugins, ordered plugin groups, dependency checks, and deferred finish hooks.
Plugins configure an `engine_app::App` synchronously when added.
Allowed workspace dependency: `engine_app`.
It does not provide dynamic loading or parallel plugin initialization.
