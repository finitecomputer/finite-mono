const $ = (id) => document.getElementById(id);

function authHeader() {
  return $("authHeader").value.trim();
}

function brainId() {
  return $("brainId").value.trim() || "smoke";
}

function folderId() {
  return $("folderId").value.trim() || "general";
}

function objectId() {
  return $("objectId").value.trim() || "obj_000000000001";
}

function inviteCode() {
  return $("inviteCode").value.trim();
}

function shareLinkId() {
  return $("shareLinkId").value.trim();
}

function sharedInvitationId() {
  return $("sharedInvitationId").value.trim();
}

function connectionId() {
  return $("connectionId").value.trim();
}

function show(value) {
  $("output").textContent =
    typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function headers(hasBody) {
  const result = {};
  const auth = authHeader();
  if (auth) result.Authorization = auth;
  if (hasBody) result["Content-Type"] = "application/json";
  return result;
}

async function request(path, options = {}) {
  const hasBody = typeof options.body === "string" && options.body.length > 0;
  const response = await fetch(path, {
    method: options.method || "GET",
    headers: headers(hasBody),
    body: hasBody ? options.body : undefined,
  });
  const text = await response.text();
  let body = text;
  try {
    body = JSON.parse(text);
  } catch (_) {
    body = text;
  }
  if (!response.ok) {
    throw { status: response.status, body };
  }
  return body;
}

function setList(id, values, empty) {
  const list = $(id);
  list.replaceChildren();
  if (!values.length) {
    const li = document.createElement("li");
    li.textContent = empty;
    list.appendChild(li);
    return;
  }
  for (const value of values) {
    const li = document.createElement("li");
    li.textContent = value;
    list.appendChild(li);
  }
}

function appendList(id, value) {
  const list = $(id);
  if (list.children.length === 1 && list.children[0].dataset.empty === "true") {
    list.replaceChildren();
  }
  const li = document.createElement("li");
  li.textContent = value;
  list.prepend(li);
}

function emptyList(id, text) {
  const list = $(id);
  list.replaceChildren();
  const li = document.createElement("li");
  li.dataset.empty = "true";
  li.textContent = text;
  list.appendChild(li);
}

function renderMetadata(metadata) {
  setList(
    "summaryList",
    [
      `${metadata.brainId} (${metadata.kind})`,
      `${(metadata.members || []).length} members / ${(metadata.admins || []).length} admins`,
      `${metadata.grantCount || 0} visible grants`,
    ],
    "No brain loaded"
  );
  setList(
    "folderList",
    (metadata.folders || []).map((folder) => {
      const setup = folder.setupIncomplete ? " setup incomplete" : "";
      return `${folder.path} (${folder.access}, v${folder.currentKeyVersion}${setup})`;
    }),
    "No folders loaded"
  );
  setList(
    "grantList",
    (metadata.folders || []).map((folder) => {
      const users = (folder.accessUserIds || []).length;
      const setup = folder.setupIncomplete ? "missing grant/setup" : "ready";
      return `${folder.id}: ${users} users, ${setup}`;
    }),
    "No grant state loaded"
  );
  renderMounts(metadata.mountedFolders || []);
}

function renderSync(sync) {
  setList(
    "summaryList",
    [
      `${sync.brainId} sync`,
      `latest sequence ${sync.latestSequence || 0}`,
      `${sync.objectCount || 0} current objects`,
    ],
    "No sync loaded"
  );
  setList(
    "objectList",
    (sync.objects || []).map((object) => {
      const deleted = object.deleted ? " deleted" : "";
      return `${object.folderId}/${object.objectId} r${object.revision}${deleted}`;
    }),
    `No objects at sequence ${sync.latestSequence || 0}`
  );
}

function renderExport(exported) {
  setList(
    "grantList",
    (exported.keyGrants || []).map((grant) => {
      return `${grant.folderId} v${grant.keyVersion} -> ${grant.recipientNpub}`;
    }),
    "No grants in export"
  );
  setList(
    "objectList",
    (exported.objects || []).map((object) => {
      const visibility = object.opaque ? "opaque" : "accessible";
      return `${object.folderId}/${object.objectId} r${object.revision} ${visibility}`;
    }),
    "No objects in export"
  );
}

function renderMounts(mounts) {
  setList(
    "mountList",
    (mounts || []).map((mount) => {
      return `${mount.displayName} -> ${mount.sourceBrainId}/${mount.sourceFolderId} (${mount.state}, ${mount.connectionId})`;
    }),
    "No mounts loaded"
  );
}

function rememberLifecycle(result) {
  if (!result || typeof result !== "object") return;
  if (result.inviteCode) {
    $("inviteCode").value = result.inviteCode;
    appendList("invitationList", `brain invitation ${result.id} ${result.status}`);
  }
  if (result.recipientNpub && result.folderId && result.acceptPath) {
    $("shareLinkId").value = result.id;
    appendList("invitationList", `Folder Invitation ${result.id} ${result.status}`);
  }
  if (result.sourceBrainId && result.destinationBrainId && result.acceptPath) {
    $("sharedInvitationId").value = result.id;
    appendList("invitationList", `Mount Offer ${result.id} ${result.status}`);
  }
  if (result.participantNpubs) {
    $("connectionId").value = result.id;
    appendList(
      "mountList",
      `Mount ${result.id} ${result.status} (${result.participantNpubs.length} participants)`
    );
  }
}

async function run(label, action) {
  show(`${label}...`);
  try {
    const result = await action();
    show(result);
    rememberLifecycle(result);
    return result;
  } catch (error) {
    show(error);
    return null;
  }
}

$("healthButton").addEventListener("click", () =>
  run("Checking health", () => request("/health"))
);

$("bootstrapButton").addEventListener("click", () =>
  run("Loading bootstrap summary", () => request("/smoke/bootstrap"))
);

$("metadataButton").addEventListener("click", async () => {
  const result = await run("Loading metadata", () =>
    request(`/v1/brains/${encodeURIComponent(brainId())}/metadata`)
  );
  if (result) renderMetadata(result);
});

$("syncButton").addEventListener("click", async () => {
  const result = await run("Loading sync bootstrap", () =>
    request(`/v1/brains/${encodeURIComponent(brainId())}/sync/bootstrap`)
  );
  if (result) renderSync(result);
});

$("mountsButton").addEventListener("click", async () => {
  const result = await run("Loading mounts", () =>
    request(`/v1/brains/${encodeURIComponent(brainId())}/mounts`)
  );
  if (result) renderMounts(result);
});

$("exportButton").addEventListener("click", async () => {
  const result = await run("Loading encrypted export", () =>
    request(`/v1/brains/${encodeURIComponent(brainId())}/export`)
  );
  if (result) renderExport(result);
});

$("searchButton").addEventListener("click", () =>
  run("Checking search privacy boundary", () =>
    request(`/v1/brains/${encodeURIComponent(brainId())}/search?q=smoke`)
  )
);

$("createBrainButton").addEventListener("click", () =>
  run("Creating brain", () =>
    request("/v1/brains", {
      method: "POST",
      body: $("createBrainBody").value,
    })
  )
);

$("createFolderButton").addEventListener("click", async () => {
  const result = await run("Creating folder", () =>
    request(`/v1/brains/${encodeURIComponent(brainId())}/folders`, {
      method: "POST",
      body: $("createFolderBody").value,
    })
  );
  if (result) renderMetadata(result);
});

$("putObjectButton").addEventListener("click", () =>
  run("Putting object", () =>
    request(
      `/v1/brains/${encodeURIComponent(brainId())}/folders/${encodeURIComponent(
        folderId()
      )}/objects/${encodeURIComponent(objectId())}`,
      {
        method: "PUT",
        body: $("syncPayload").value,
      }
    )
  )
);

$("getObjectButton").addEventListener("click", () =>
  run("Getting object", () =>
    request(
      `/v1/brains/${encodeURIComponent(brainId())}/folders/${encodeURIComponent(
        folderId()
      )}/objects/${encodeURIComponent(objectId())}`
    )
  )
);

$("submitSyncButton").addEventListener("click", () =>
  run("Submitting sync record", () =>
    request(`/v1/brains/${encodeURIComponent(brainId())}/sync/records`, {
      method: "POST",
      body: $("syncPayload").value,
    })
  )
);

$("createBrainInvitationButton").addEventListener("click", () =>
  run("Creating brain invitation", () =>
    request(`/v1/brains/${encodeURIComponent(brainId())}/invitations`, {
      method: "POST",
      body: $("brainInvitationBody").value,
    })
  )
);

$("getBrainInvitationButton").addEventListener("click", () =>
  run("Getting brain invitation", () =>
    request(`/v1/brain-invitation-links/${encodeURIComponent(inviteCode())}`)
  )
);

$("acceptBrainInvitationButton").addEventListener("click", () =>
  run("Accepting brain invitation", () =>
    request(`/v1/brain-invitation-links/${encodeURIComponent(inviteCode())}/accept`, {
      method: "POST",
    })
  )
);

$("createShareLinkButton").addEventListener("click", () =>
  run("Creating Folder invitation", () =>
    request(
      `/v1/brains/${encodeURIComponent(brainId())}/folders/${encodeURIComponent(
        folderId()
      )}/invitations`,
      {
        method: "POST",
        body: $("shareLinkBody").value,
      }
    )
  )
);

$("getShareLinkButton").addEventListener("click", () =>
  run("Getting Folder invitation", () =>
    request(`/v1/invitations/${encodeURIComponent(shareLinkId())}`)
  )
);

$("acceptShareLinkButton").addEventListener("click", () =>
  run("Accepting Folder invitation", () =>
    request(`/v1/invitations/${encodeURIComponent(shareLinkId())}/accept`, {
      method: "POST",
    })
  )
);

$("revokeShareLinkButton").addEventListener("click", () =>
  run("Revoking Folder invitation", () =>
    request(`/v1/invitations/${encodeURIComponent(shareLinkId())}`, {
      method: "DELETE",
    })
  )
);

$("createSharedInvitationButton").addEventListener("click", () =>
  run("Creating mount offer", () =>
    request(
      `/v1/brains/${encodeURIComponent(brainId())}/folders/${encodeURIComponent(
        folderId()
      )}/mount-offers`,
      {
        method: "POST",
        body: $("sharedFolderBody").value,
      }
    )
  )
);

$("getSharedInvitationButton").addEventListener("click", () =>
  run("Getting mount offer", () =>
    request(`/v1/mount-offers/${encodeURIComponent(sharedInvitationId())}`)
  )
);

$("acceptSharedInvitationButton").addEventListener("click", () =>
  run("Accepting mount offer", () =>
    request(`/v1/mount-offers/${encodeURIComponent(sharedInvitationId())}/accept`, {
      method: "POST",
    })
  )
);

$("revokeConnectionButton").addEventListener("click", () =>
  run("Revoking mount", () =>
    request(`/v1/mounts/${encodeURIComponent(connectionId())}`, {
      method: "DELETE",
      body: $("sharedFolderBody").value,
    })
  )
);

emptyList("summaryList", "No brain loaded");
emptyList("folderList", "No folders loaded");
emptyList("objectList", "No sync state loaded");
emptyList("grantList", "No grant state loaded");
emptyList("invitationList", "No Brain or Folder Invitations loaded");
emptyList("mountList", "No connections or mounts loaded");
