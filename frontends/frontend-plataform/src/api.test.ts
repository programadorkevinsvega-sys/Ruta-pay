import { describe, expect, it } from "vitest";
import { ApiError, correlationId, userMessage } from "./api";

describe("safe API diagnostics", () => {
  it("creates a standards-shaped correlation identifier", () => {
    expect(correlationId()).toMatch(/^[0-9a-f-]{36}$/i);
  });

  it("keeps the BFF's safe message and correlation reference", () => {
    const diagnostic = userMessage(new ApiError(401, "authentication_failed", "corr-42", "No se pudo completar la solicitud."));
    expect(diagnostic).toEqual({ message: "No se pudo completar la solicitud.", correlationId: "corr-42" });
  });

  it("does not expose arbitrary thrown values", () => {
    expect(userMessage(new Error("secret response body"))).toEqual({
      message: "No se pudo conectar con la plataforma.",
      correlationId: "",
    });
  });
});
