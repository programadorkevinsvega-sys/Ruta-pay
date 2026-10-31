export function merchantSlugFromName(name: string): string {
  return name
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64)
    .replace(/-+$/g, "");
}

export function passwordRequirements(password: string) {
  const characters = Array.from(password);
  return {
    length: characters.length >= 12 && characters.length <= 128,
    uppercase: characters.some((character) => /[A-Z]/.test(character)),
    lowercase: characters.some((character) => /[a-z]/.test(character)),
    number: characters.some((character) => /[0-9]/.test(character)),
    symbol: characters.some((character) => {
      const code = character.charCodeAt(0);
      return code >= 33 && code <= 126 && !/[A-Za-z0-9]/.test(character);
    }),
  };
}

export function isPasswordValid(password: string): boolean {
  return Object.values(passwordRequirements(password)).every(Boolean);
}

export function isLegalIdentificationNumberValid(value: string): boolean {
  const digits = value.replace(/[ .-]/g, "");
  return /^[0-9]{5,20}$/.test(digits);
}
