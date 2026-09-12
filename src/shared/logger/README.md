# Logger

Console, JSON and no-op output with explicit projections and bounded delivery.

Read the [contract](CONTRACT.md), then the native config, projection, delivery, runtime/handler files.
The composition root supplies settings, resources and lifecycle; this module never
imports root or product code.

- [Implementation notes](../../../notes/modules/src/shared/logger/README.md).
- [Runnable example](../../../FOUNDATIONS.md).

Config::new supplies console/info/auto and queue defaults. An explicitly supplied zero capacity or record bound is refused. Native required capabilities are represented by owned nonoptional types.
