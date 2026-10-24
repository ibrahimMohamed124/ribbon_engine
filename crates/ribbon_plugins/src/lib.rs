//! Synchronous plugin registration and dependency validation for [`ribbon_app`].

use ribbon_app::App;
use std::fmt;

/// The names of plugins already added to an app, in registration order.
#[derive(Clone, Debug, Default)]
pub struct Plugins {
    /// Registered plugin names.
    pub names: Vec<&'static str>,
}

/// A unit of engine or game functionality that configures an app.
pub trait Plugin: 'static {
    /// Stable diagnostic name.
    fn name(&self) -> &'static str;

    /// Plugins that must already have been added.
    fn dependencies(&self) -> &'static [&'static str] {
        &[]
    }

    /// Configure resources and systems immediately when added.
    fn build(&self, app: &mut App);

    /// Finalize after every requested plugin has been added.
    fn finish(&self, _app: &mut App) {}
}

/// An ordered collection of plugins.
pub trait PluginGroup {
    /// Return plugins in dependency-compatible registration order.
    fn plugins(self) -> Vec<Box<dyn Plugin>>;
}

/// Plugin registration errors include both the dependent and required plugin names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginError {
    /// A plugin with this name is already registered.
    Duplicate { plugin: &'static str },
    /// The required plugin has not been added yet.
    MissingDependency {
        plugin: &'static str,
        dependency: &'static str,
    },
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { plugin } => write!(f, "plugin `{plugin}` was added more than once"),
            Self::MissingDependency { plugin, dependency } => write!(
                f,
                "plugin `{plugin}` requires `{dependency}` to be added first"
            ),
        }
    }
}

impl std::error::Error for PluginError {}

/// Extension methods that add plugins to an app in dependency order.
pub trait AppPluginExt {
    /// Build and register one plugin immediately.
    fn add_plugin<P: Plugin>(&mut self, plugin: P) -> Result<&mut Self, PluginError>;

    /// Add every plugin in the group's returned order.
    fn add_plugins<G: PluginGroup>(&mut self, group: G) -> Result<&mut Self, PluginError>;
}

impl AppPluginExt for App {
    fn add_plugin<P: Plugin>(&mut self, plugin: P) -> Result<&mut Self, PluginError> {
        let name = plugin.name();
        let registered = self
            .world
            .try_resource::<Plugins>()
            .map(|plugins| &plugins.names);
        if registered.is_some_and(|names| names.contains(&name)) {
            return Err(PluginError::Duplicate { plugin: name });
        }
        for &dependency in plugin.dependencies() {
            if !registered.is_some_and(|names| names.contains(&dependency)) {
                return Err(PluginError::MissingDependency {
                    plugin: name,
                    dependency,
                });
            }
        }

        plugin.build(self);
        if let Some(plugins) = self.world.try_resource::<Plugins>() {
            let mut names = plugins.names.clone();
            names.push(name);
            self.world.insert_resource(Plugins { names });
        } else {
            self.world.insert_resource(Plugins { names: vec![name] });
        }
        self.add_finish_hook(Box::new(move |app| plugin.finish(app)));
        Ok(self)
    }

    fn add_plugins<G: PluginGroup>(&mut self, group: G) -> Result<&mut Self, PluginError> {
        for plugin in group.plugins() {
            self.add_boxed_plugin(plugin)?;
        }
        Ok(self)
    }
}

// Keep trait-object group entries on the same validation path as generic additions.
trait AddBoxedPlugin {
    fn add_boxed_plugin(&mut self, plugin: Box<dyn Plugin>) -> Result<(), PluginError>;
}

impl AddBoxedPlugin for App {
    fn add_boxed_plugin(&mut self, plugin: Box<dyn Plugin>) -> Result<(), PluginError> {
        let name = plugin.name();
        let registered = self
            .world
            .try_resource::<Plugins>()
            .map(|plugins| &plugins.names);
        if registered.is_some_and(|names| names.contains(&name)) {
            return Err(PluginError::Duplicate { plugin: name });
        }
        for &dependency in plugin.dependencies() {
            if !registered.is_some_and(|names| names.contains(&dependency)) {
                return Err(PluginError::MissingDependency {
                    plugin: name,
                    dependency,
                });
            }
        }
        plugin.build(self);
        let mut names = self
            .world
            .try_resource::<Plugins>()
            .map_or_else(Vec::new, |plugins| plugins.names.clone());
        names.push(name);
        self.world.insert_resource(Plugins { names });
        self.add_finish_hook(Box::new(move |app| plugin.finish(app)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AppPluginExt, Plugin, PluginError, PluginGroup, Plugins};
    use ribbon_app::App;
    use std::sync::{Arc, Mutex};

    struct NamedPlugin {
        name: &'static str,
        dependencies: &'static [&'static str],
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Plugin for NamedPlugin {
        fn name(&self) -> &'static str {
            self.name
        }

        fn dependencies(&self) -> &'static [&'static str] {
            self.dependencies
        }

        fn build(&self, _app: &mut App) {
            self.log.lock().unwrap().push(self.name);
        }

        fn finish(&self, _app: &mut App) {
            self.log.lock().unwrap().push("finish");
        }
    }

    struct OrderedGroup(Arc<Mutex<Vec<&'static str>>>);

    impl PluginGroup for OrderedGroup {
        fn plugins(self) -> Vec<Box<dyn Plugin>> {
            vec![
                Box::new(NamedPlugin {
                    name: "base",
                    dependencies: &[],
                    log: Arc::clone(&self.0),
                }),
                Box::new(NamedPlugin {
                    name: "dependent",
                    dependencies: &["base"],
                    log: Arc::clone(&self.0),
                }),
            ]
        }
    }

    #[test]
    fn dependencies_are_checked_and_duplicate_plugins_are_rejected() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        let dependent = NamedPlugin {
            name: "dependent",
            dependencies: &["base"],
            log: Arc::clone(&log),
        };
        assert!(matches!(
            app.add_plugin(dependent),
            Err(PluginError::MissingDependency {
                plugin: "dependent",
                dependency: "base"
            })
        ));
        app.add_plugin(NamedPlugin {
            name: "base",
            dependencies: &[],
            log: Arc::clone(&log),
        })
        .unwrap();
        assert!(matches!(
            app.add_plugin(NamedPlugin {
                name: "base",
                dependencies: &[],
                log
            }),
            Err(PluginError::Duplicate { plugin: "base" })
        ));
        assert_eq!(app.world.resource::<Plugins>().names, ["base"]);
    }

    #[test]
    fn groups_build_in_order_and_finish_after_registration() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        app.add_plugins(OrderedGroup(Arc::clone(&log))).unwrap();
        assert_eq!(*log.lock().unwrap(), ["base", "dependent"]);
        assert_eq!(app.world.resource::<Plugins>().names, ["base", "dependent"]);
        let _app = app.build().unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            ["base", "dependent", "finish", "finish"]
        );
    }
}
