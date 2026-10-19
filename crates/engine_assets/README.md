# engine_assets
Provides synchronous asset sources, typed weak handles, extension loaders, caching, and polling hot reload.
`BytesLoader` and `TextLoader` cover common unprocessed data.
Allowed workspace dependencies: `engine_app`, `engine_plugins`, `engine_ecs`, and `engine_events`.
It does not perform asynchronous loading or silently replace missing assets with defaults.
