/** 把任意字串雜湊成一個穩定的 0–359 色相值，同一個字串永遠同色。 */
export function hashLabelHue(label: string): number {
  let h = 0;
  for (let i = 0; i < label.length; i++) {
    h = (h * 31 + label.charCodeAt(i)) | 0;
  }
  return Math.abs(h) % 360;
}
