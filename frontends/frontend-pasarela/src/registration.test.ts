import { describe, expect, it } from "vitest";
import { isLegalIdentificationNumberValid, isPasswordValid, merchantSlugFromName, passwordRequirements } from "./registration";

describe("merchant registration fields", () => {
  it("derives a normalized identifier from the visible merchant name", () => {
    expect(merchantSlugFromName("Café de la Ñora & Hijos")).toBe("cafe-de-la-nora-hijos");
  });

  it("limits the generated identifier to the backend length", () => {
    expect(merchantSlugFromName("a".repeat(80))).toHaveLength(64);
  });

  it("requires length, uppercase, lowercase, digit, and punctuation", () => {
    expect(isPasswordValid("Long-enough-password1!")).toBe(true);
    expect(passwordRequirements("long-enough-password1!").uppercase).toBe(false);
    expect(passwordRequirements("LONG-ENOUGH-PASSWORD1!").lowercase).toBe(false);
    expect(passwordRequirements("Long-enough-password!").number).toBe(false);
    expect(passwordRequirements("Longenoughpassword1").symbol).toBe(false);
    expect(isPasswordValid("Short1!")).toBe(false);
  });

  it("accepts formatted NIT or CC numbers only when they contain 5 to 20 digits", () => {
    expect(isLegalIdentificationNumberValid("900.123.456-7")).toBe(true);
    expect(isLegalIdentificationNumberValid("12 345")).toBe(true);
    expect(isLegalIdentificationNumberValid("12AB34")).toBe(false);
    expect(isLegalIdentificationNumberValid("1234")).toBe(false);
  });
});
