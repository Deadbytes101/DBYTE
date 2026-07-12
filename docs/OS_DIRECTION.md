# DByte OS Direction

## Evolution from Language to System

DByte started as a high-performance interpreter aiming to compete with Python in speed and simplicity. While it has achieved significant success in benchmarks, its ultimate goal has shifted.

**DByte is now evolving into a Personal Computing Environment.**

### Core Philosophy
- **Identity over Generalization**: DByte is not just a tool; it's a workspace.
- **Programmable by Default**: Every layer of the system should be accessible and modifiable by the user.
- **Immediate Feedback**: Like the spirit of TempleOS/HolyC, the distance between idea and execution should be near-zero.
- **Self-Contained**: The system should provide all essential low-level tools (hex, patch, inspect) out of the box.

### The Roadmap

| Version | Phase | Focus |
|---|---|---|
| `v2.8.0` | **Sanctum Foundation** | Documentation, Workspace conventions, .dbyterc environment |
| `v2.9.0` | **System Workspace** | Advanced userland tools, standard library expansion |
| `v3.0.0` | **Userland Prototype** | Full-screen shell, basic GUI/Visual elements on host OS |
| `v3.1.0` | **VM Host Runtime** | DByte VM as a host for other micro-services |
| `v4.0.0` | **Kernel Experiment** | Bare-metal research, standalone OS architecture |

### Sanctum over Kernel
We prioritize the **DByte Sanctum**—a userland personal environment—before attempting a bare-metal kernel. A system's soul is its userland and shell; we build the soul on host OSes first.

