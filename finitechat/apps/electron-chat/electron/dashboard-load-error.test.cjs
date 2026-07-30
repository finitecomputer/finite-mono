const assert = require("node:assert/strict");
const test = require("node:test");
const {
  dashboardLoadErrorScript,
  shouldReplaceFailedDashboardDocument,
} = require("./dashboard-load-error.cjs");

const expected = {
  currentUrl: "https://finite.computer/dashboard/machines/waffle/chat",
  dashboardWebContentsId: 17,
};

test("only a failed current dashboard main document is replaced", () => {
  const failure = {
    resourceType: "mainFrame",
    statusCode: 502,
    url: expected.currentUrl,
    webContentsId: expected.dashboardWebContentsId,
  };
  assert.equal(shouldReplaceFailedDashboardDocument(failure, expected), true);
  assert.equal(
    shouldReplaceFailedDashboardDocument({ ...failure, resourceType: "script" }, expected),
    false
  );
  assert.equal(
    shouldReplaceFailedDashboardDocument({ ...failure, statusCode: 404 }, expected),
    false
  );
  assert.equal(
    shouldReplaceFailedDashboardDocument({ ...failure, statusCode: 200 }, expected),
    false
  );
  assert.equal(
    shouldReplaceFailedDashboardDocument({ ...failure, webContentsId: 18 }, expected),
    false
  );
  assert.equal(
    shouldReplaceFailedDashboardDocument(
      { ...failure, url: "https://finite.computer/dashboard" },
      expected
    ),
    false
  );
});

test("the local failure document has one explicit retry action and no remote input", () => {
  const script = dashboardLoadErrorScript("data:image/svg+xml;base64,PHN2ZyAvPg==");
  assert.match(script, /Finite is temporarily unavailable/u);
  assert.match(script, /Your local chat data is safe/u);
  assert.match(script, /location\.reload\(\)/u);
  assert.match(script, /data:image\/svg\+xml;base64/u);
  assert.doesNotMatch(script, /https?:\/\//u);
  assert.doesNotMatch(script, /innerHTML/u);
});

test("the failure document rejects missing or remote logo assets", () => {
  assert.throws(() => dashboardLoadErrorScript(), /packaged logo/u);
  assert.throws(
    () => dashboardLoadErrorScript("https://finite.computer/finite-logo.svg"),
    /packaged logo/u
  );
});
