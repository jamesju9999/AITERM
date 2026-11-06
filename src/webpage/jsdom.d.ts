// jsdom 沒附型別；這支測試只用到 JSDOM 建構子。
declare module "jsdom" {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  export const JSDOM: any;
}
