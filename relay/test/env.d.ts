declare namespace Cloudflare {
  interface Env {
    ROOM: DurableObjectNamespace<import("../src/index").Room>;
    LIMITER: DurableObjectNamespace<import("../src/index").Limiter>;
  }
}
