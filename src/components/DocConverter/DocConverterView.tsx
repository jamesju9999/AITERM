// src/components/DocConverter/DocConverterView.tsx
export function DocConverterView({ isActive }: { isActive: boolean }) {
  if (!isActive) return null;
  return (
    <div style={{ padding: 24, color: "#e6e6e6" }}>
      Doc Converter — 建置中
    </div>
  );
}
