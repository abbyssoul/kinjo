Avahi-TUI: TUI browser for service discovery.

## What's it for?
Avahi (implementation of Bojour / mDNS/DNS-SD protocol) allows to publish and discover
services running on a local network. This TUI application allows user to see discovered services and 
configure an application launcher for services.

For example, launch TUI without arguments:

```sh
./avahi-tui
```

to see the list of local services. New services will be added to the list as they dynaically discovered.


The application is driven by user defined configuration files that specify match criteria. It comes pre-configured with a simple SSH opener:
```sh
cat ~/.config/avahi-tui/commands/ssh.toml
```

```toml
[metadata]
name = "ssh"
description = "SSH into a service"
requirements = ["ssh"]
match = 'proto in (tcp, udp), '

[action]
description = "SSH into the selected service"
command = "ssh '{hostname}:{port}'"
mode = "fork"
```

This simple config file defines a command, that is applicabe to a service of type `_ssh`, and when executed, will SSH into the selected service, using its adveritsed hostname and port.
Note that action includes mode = "fork": Runs the command and returns to TUI when done.
Supported action modes:

 - fork: Run command, return to TUI afterward. Used when you want to launch a browser, for example.
 - execute: Replace TUI with the command (doesn't return). User when you want to replace TUI instance with another terminal command.


### Configuration
Configutaion files follow  XDG Base Directory Specification for configuration files. For local changes,
user can add new config files into `$XDG_CONFIG_HOME` location (default is `~/.config/avahi-tui/`).

### Service matcher

When writing a customer config, a user required to specify an expression used to match a service record to that action.
Such expression operates on a set of attributes of a service record:
 - name: Name of a service
 - type: Service type
 - domain: Doamin where the service is registered
 - hostname: Resolved name of the host where the service is advertised.
 - address: IP address of the service
 - port: Port on which the service is available.
 - txt: Text entry that the service advertised.

Note all the fields above can be used in match expression as well as `action` section for command interpolation.


Multiple configured actions can match the same service record. In that case TUI gives user an option to select what action should be taken.
