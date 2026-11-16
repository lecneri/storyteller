// Em todo campo de envio de imagem visível, junta a área para colar/arrastar (St.zonaColar).
// Campos escondidos (ex.: "Descrever pela foto") chamam St.zonaColar por conta própria.
document.querySelectorAll('input[type=file]:not([hidden]):not([data-sem-colar])').forEach((input) => window.St.zonaColar(input));
