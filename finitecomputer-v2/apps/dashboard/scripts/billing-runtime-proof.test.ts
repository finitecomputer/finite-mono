import test from "node:test";
import assert from "node:assert/strict";
import { assertRecovered, localOrigin, ownsContainer, type RuntimeProof } from "./billing-runtime-proof";
const before: RuntimeProof = {project:"p",runtime:"r",principal:"npub",room:"room",topic:"home",chat:"chat",fileHash:"sha",messageIds:["sent","reply"],running:true,startedAt:"one"};
const stopped = {...before,running:false};
const after = {...before,startedAt:"two",messageIds:["sent","reply","new"]};
test("requires actual stop and restart, same durable identity and ordered history", () => {
  assertRecovered(before, stopped, after);
  for (const patch of [{running:false},{startedAt:"one"},{principal:"other"},{fileHash:"changed"},{messageIds:["reply","sent"]},{messageIds:["sent"]}]) {
    assert.throws(() => assertRecovered(before, stopped, {...after,...patch}));
  }
  assert.throws(() => assertRecovered(before, before, after));
});
test("refuses production application endpoints", () => {
  assert.equal(localOrigin("http://127.0.0.1:123/dashboard"),"http://127.0.0.1:123");
  for(const url of ["https://finite.computer", "http://user:pass@localhost", "https://localhost"]) assert.throws(()=>localOrigin(url));
});
test("cleanup finds interrupted enrollment and exact probe without saved project; retains unrelated data", () => {
  const run = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
  const owned = {Name:"/agent",Config:{Image:"pin",Labels:{"computer.finite.v2.project_id":"p"}},Mounts:[{Type:"bind",Source:"/tmp/run/stack/runs/default/runner/agent"}]};
  assert(ownsContainer(owned,"pin","/tmp/run",run));
  assert(ownsContainer({...owned,Name:`/billing-${run}-host-network-probe`,Mounts:[]},"pin","/tmp/run",run));
  assert(!ownsContainer({...owned,Mounts:[{Type:"bind",Source:"/tmp/run/stack-other/agent"}]},"pin","/tmp/run",run));
  assert(!ownsContainer({...owned,Mounts:[{Type:"bind",Source:"/tmp/run/stack/../../other"}]},"pin","/tmp/run",run));
  assert(!ownsContainer(owned,"other-image","/tmp/run",run));
  assert(!ownsContainer({...owned,Config:{Image:"pin"}},"pin","/tmp/run",run));
});
