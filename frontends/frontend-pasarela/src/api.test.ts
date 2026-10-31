import { describe, expect, it } from "vitest";
import { ApiError, correlationId, userMessage } from "./api";

describe("safe API diagnostics", () => {
  it("generates a correlation identifier", () => {
    expect(correlationId()).toMatch(/^[0-9a-f-]{36}$/i);
  });

  it("preserves only the safe BFF message and correlation reference", () => {
    expect(userMessage(new ApiError(401, "authentication_failed", "corr-42", "No se pudo completar la solicitud."))).toEqual({
      message: "No se pudo completar la solicitud.",
      correlationId: "corr-42",
    });
  });

  it("does not display arbitrary exception contents", () => {
    expect(userMessage(new Error("private backend data"))).toEqual({
      message: "No se pudo conectar con la plataforma.",
      correlationId: "",
    });
  });
});
