import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { afterEach, beforeEach } from "@jest/globals";

const originalAntexHome = process.env.ANTEX_HOME;
let currentAntexHome: string | undefined;

beforeEach(async () => {
  currentAntexHome = await fs.mkdtemp(path.join(os.tmpdir(), "antex-sdk-test-"));
  process.env.ANTEX_HOME = currentAntexHome;
});

afterEach(async () => {
  const antexHomeToDelete = currentAntexHome;
  currentAntexHome = undefined;

  if (originalAntexHome === undefined) {
    delete process.env.ANTEX_HOME;
  } else {
    process.env.ANTEX_HOME = originalAntexHome;
  }

  if (antexHomeToDelete) {
    await fs.rm(antexHomeToDelete, { recursive: true, force: true });
  }
});
