// Reference application of the Socket.IO gateway scenarios: gateways written for the original framework with its
// Socket.IO platform adapter (test-only packages, pinned in package.json and package-lock.json). The Corvis side is the
// same application written with `#[websocket_gateway]` (crates/corvis/tests/engine_support/gateway_app.rs); change one
// and you change the other. Plain JavaScript, with the decorators applied as functions, so there is no build step.
//
//   node gateway-server.mjs
//
// Binds to 127.0.0.1 on a port chosen by the operating system and prints `{"address":...,"port":N}` as its first stdout
// line. Exits on SIGTERM and when stdin closes. What happened on the server side is served to loopback clients by
// `GET /harness/log` and cleared by `POST /harness/reset`.
//
// Gateways (Engine.IO path `/socket.io/` unless stated):
//   /            hooks that log, and every kind of handler: echo, silent, nothing, reply, duck, falsy, args, whoami, join,
//                shout, fail-ws, fail-ws-object, fail-http, fail-unknown, mark (the marker event), upload, unknown events
//   /rules       a gateway guard (`auth.token` must be set), a validation pipe, an interceptor and two exception filters
//   /mw          connect middleware registered in afterInit (`auth.token` "no" is refused) and hooks that log
//   /a, /b       two gateways, two namespaces of one server
//   path /cors-one and /cors-any: `cors` declared on the gateway
import "reflect-metadata";
import { BadRequestException, Catch, Controller, Get, HttpCode, Injectable, Module, Post, Req, UseFilters, UseGuards, UseInterceptors } from "@nestjs/common";
import { NestFactory } from "@nestjs/core";
import { IoAdapter } from "@nestjs/platform-socket.io";
import { ConnectedSocket, MessageBody, SubscribeMessage, WebSocketGateway, WebSocketServer, WsException } from "@nestjs/websockets";
import { map } from "rxjs/operators";

const log = [];

function record(entry) {
  log.push(entry);
}

// ------------------------------------------------------------------------------------------------ decorators as functions

function handler(Class, name, event, params = []) {
  const descriptor = Object.getOwnPropertyDescriptor(Class.prototype, name);
  SubscribeMessage(event)(Class.prototype, name, descriptor);
  params.forEach((decorator, index) => decorator(Class.prototype, name, index));
}

function server(Class, property) {
  WebSocketServer()(Class.prototype, property);
}

// -------------------------------------------------------------------------------------------------------------- enhancers

class NeedsToken {
  canActivate(context) {
    const client = context.switchToWs().getClient();
    record({
      event: "guard",
      type: context.getType(),
      class: context.getClass().name,
      handler: context.getHandler().name,
      data: context.switchToWs().getData() ?? null,
    });
    return client.handshake.auth?.token !== undefined;
  }
}
Injectable()(NeedsToken);

class NumberPipe {
  transform(value) {
    if (typeof value !== "number") throw new WsException("not a number");
    return value;
  }
}

class WrapInterceptor {
  intercept(_context, next) {
    return next.handle().pipe(map((value) => ({ wrapped: value })));
  }
}
Injectable()(WrapInterceptor);

class RoomError extends Error {}

class RoomErrorFilter {
  catch(_exception, host) {
    host.switchToWs().getClient().emit("exception", { by: "room-filter" });
  }
}
Catch(RoomError)(RoomErrorFilter);

class QuietError extends Error {}

class QuietFilter {
  catch() {}
}
Catch(QuietError)(QuietFilter);

// ------------------------------------------------------------------------------------------------------------------ gateways

class MainGateway {
  afterInit(server) {
    record({ event: "init", nsp: server.name ?? "/" });
  }
  handleConnection(client) {
    record({ event: "connection", nsp: client.nsp.name, auth: client.handshake.auth });
  }
  handleDisconnect(client) {
    record({ event: "disconnect", nsp: client.nsp.name });
  }
  echo(body) {
    record({ event: "message", handler: "echo" });
    return body ?? null;
  }
  silent() {}
  nothing() {
    return null;
  }
  reply() {
    return { event: "reply", data: 1 };
  }
  duck() {
    return { event: "duck", data: 2 };
  }
  falsy() {
    return 0;
  }
  args(body) {
    // No argument is `undefined` in JavaScript and `null` in Corvis: the application states it.
    return { body: body === undefined ? null : body };
  }
  whoami(client) {
    const rooms = [...client.rooms].filter((room) => room !== client.id).sort();
    return { idLength: client.id.length, rooms, auth: client.handshake.auth, nsp: client.nsp.name };
  }
  join(client, room) {
    client.join(room);
  }
  shout(room) {
    this.server.to(room).emit("news", "hello");
  }
  failWs() {
    throw new WsException("Room is full");
  }
  failWsObject() {
    throw new WsException({ code: "ROOM_FULL" });
  }
  failHttp() {
    throw new BadRequestException("bad input");
  }
  failUnknown() {
    throw new Error("secret database detail");
  }
  mark() {
    return { event: "done", data: true };
  }
  upload(body) {
    return { body: body === undefined ? null : body };
  }
}
WebSocketGateway()(MainGateway);
server(MainGateway, "server");
handler(MainGateway, "echo", "echo", [MessageBody()]);
handler(MainGateway, "silent", "silent");
handler(MainGateway, "nothing", "nothing");
handler(MainGateway, "reply", "reply");
handler(MainGateway, "duck", "duck");
handler(MainGateway, "falsy", "falsy");
handler(MainGateway, "args", "args", [MessageBody()]);
handler(MainGateway, "whoami", "whoami", [ConnectedSocket()]);
handler(MainGateway, "join", "join", [ConnectedSocket(), MessageBody("room")]);
handler(MainGateway, "shout", "shout", [MessageBody("room")]);
handler(MainGateway, "failWs", "fail-ws");
handler(MainGateway, "failWsObject", "fail-ws-object");
handler(MainGateway, "failHttp", "fail-http");
handler(MainGateway, "failUnknown", "fail-unknown");
handler(MainGateway, "mark", "mark");
handler(MainGateway, "upload", "upload", [MessageBody()]);

class RulesGateway {
  guarded() {
    return "ok";
  }
  piped(n) {
    return { n };
  }
  wrapped() {
    return 7;
  }
  failRoom() {
    throw new RoomError("room");
  }
  failQuiet() {
    throw new QuietError("quiet");
  }
  mark() {
    return { event: "done", data: true };
  }
}
WebSocketGateway({ namespace: "/rules" })(RulesGateway);
UseGuards(NeedsToken)(RulesGateway);
UseFilters(RoomErrorFilter, QuietFilter)(RulesGateway);
handler(RulesGateway, "guarded", "guarded");
handler(RulesGateway, "piped", "piped", [MessageBody(new NumberPipe())]);
handler(RulesGateway, "wrapped", "wrapped");
UseInterceptors(WrapInterceptor)(RulesGateway.prototype, "wrapped", Object.getOwnPropertyDescriptor(RulesGateway.prototype, "wrapped"));
handler(RulesGateway, "failRoom", "fail-room");
handler(RulesGateway, "failQuiet", "fail-quiet");
handler(RulesGateway, "mark", "mark");

class MiddlewareGateway {
  afterInit(namespace) {
    namespace.use((socket, next) => {
      if (socket.handshake.auth?.token === "no") next(new Error("not allowed"));
      else next();
    });
  }
  handleConnection(client) {
    record({ event: "connection", nsp: client.nsp.name, auth: client.handshake.auth });
  }
  hello() {
    return "hi";
  }
}
WebSocketGateway({ namespace: "/mw" })(MiddlewareGateway);
handler(MiddlewareGateway, "hello", "hello");

class AGateway {
  ping() {
    return "a";
  }
}
WebSocketGateway({ namespace: "/a" })(AGateway);
handler(AGateway, "ping", "ping");

class BGateway {
  ping() {
    return "b";
  }
}
WebSocketGateway({ namespace: "/b" })(BGateway);
handler(BGateway, "ping", "ping");

class CorsOneGateway {}
WebSocketGateway({ path: "/cors-one", cors: { origin: "https://app.example" } })(CorsOneGateway);

class CorsAnyGateway {}
WebSocketGateway({ path: "/cors-any", cors: true })(CorsAnyGateway);

// ---------------------------------------------------------------------------------------------------------- control routes

class HarnessController {
  log(request) {
    return loopback(request) ? log : { error: "forbidden" };
  }
  reset(request) {
    // What happened at startup (`init`) stays: it cannot happen again.
    if (loopback(request)) log.splice(0, log.length, ...log.filter((entry) => entry.event === "init"));
  }
}
function loopback(request) {
  return ["127.0.0.1", "::1", "::ffff:127.0.0.1"].includes(request.socket.remoteAddress);
}
Controller("harness")(HarnessController);
Get("log")(HarnessController.prototype, "log", Object.getOwnPropertyDescriptor(HarnessController.prototype, "log"));
Req()(HarnessController.prototype, "log", 0);
Post("reset")(HarnessController.prototype, "reset", Object.getOwnPropertyDescriptor(HarnessController.prototype, "reset"));
HttpCode(204)(HarnessController.prototype, "reset", Object.getOwnPropertyDescriptor(HarnessController.prototype, "reset"));
Req()(HarnessController.prototype, "reset", 0);

class AppModule {}
Module({
  controllers: [HarnessController],
  providers: [MainGateway, RulesGateway, MiddlewareGateway, AGateway, BGateway, CorsOneGateway, CorsAnyGateway],
})(AppModule);

// ---------------------------------------------------------------------------------------------------------------- startup

const app = await NestFactory.create(AppModule, { logger: false });
app.useWebSocketAdapter(new IoAdapter(app));
await app.listen(0, "127.0.0.1");
const { address, port } = app.getHttpServer().address();
console.log(JSON.stringify({ address, port }));

async function stop() {
  await app.close();
  process.exit(0);
}
process.on("SIGTERM", stop);
process.stdin.on("end", stop);
process.stdin.resume();
