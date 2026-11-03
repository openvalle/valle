// error: errors/font-uncovered.tsx:6:10: 'ภ' (U+0E20): no font has it; declare a font resource that does, such as one for Thai or Arabic text
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

// Thai, which no built-in font has: a font resource must give it.
export default function Scene() {
  return <Text>ภาษาไทย</Text>;
}
