# Native socket cleanup

The Axum adapter transfers the upgraded stream into a Tokio task. select! coordinates incoming frames, heartbeat and shutdown; timeout drops the owned handler future. Hyper must enable with_upgrades() or a valid-looking upgrade route will never become a usable socket. Native capacity errors needed an explicit 1009 close frame; merely returning the error dropped TCP without the agreed close code.

Root stops new HTTP admission while draining sockets under the same remaining
shutdown budget. A graceful 1001 close is attempted before forced transport close.
The root diagnostic's handler never retains a socket handle. A future domain
subscription must own its cleanup explicitly rather than capture a connection in
an unbounded background task.
