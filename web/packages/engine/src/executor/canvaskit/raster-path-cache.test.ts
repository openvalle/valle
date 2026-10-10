import { expect, test } from "bun:test";
import type { Path } from "canvaskit-wasm";
import { RasterPathCache } from "./raster-path-cache.ts";

test("fill rules remain distinct and eviction retires every native object", () => {
  let disposed=0;const create=()=>({delete(){disposed++;}} as unknown as Path);
  const wire={verbs:['moveTo'],points:[[0,0] as [number,number]]},cache=new RasterPathCache(400);
  const a=cache.acquire(wire,'nonZero',create);
  expect(cache.acquire(wire,'nonZero',create).path).toBe(a.path);
  expect(cache.acquire(wire,'evenOdd',create).path).not.toBe(a.path);
  expect(a.owned).toBe(false);
  cache.acquire({verbs:['moveTo'],points:[[1,1]]},'nonZero',create);
  expect(disposed).toBe(2);cache.dispose();expect(disposed).toBe(3);
});

test("an oversized native path has an explicit caller-owned lease",()=>{
  const cache=new RasterPathCache(1),path={delete(){}} as unknown as Path;
  expect(cache.acquire({verbs:[],points:[]},'nonZero',()=>path)).toEqual({path,owned:true});cache.dispose();
});

test("unchanged contours share a native path across independently decoded arenas", () => {
  let created = 0, deleted = 0;
  const cache = new RasterPathCache();
  const create = () => {
    created++;
    return {delete() {deleted++;}} as unknown as Path;
  };
  const first = {verbs: ["moveTo", "lineTo"], points: [[1, 2], [3, 4]] as Array<[number, number]>};
  const a = cache.acquire(first, "nonZero", create);
  const b = cache.acquire(JSON.parse(JSON.stringify(first)), "nonZero", create);
  expect(b.path).toBe(a.path);
  expect(created).toBe(1);
  expect(cache.acquire({verbs: first.verbs, points: [[1, 2], [3, 5]]}, "nonZero", create).path).not.toBe(a.path);
  cache.dispose();
  expect(deleted).toBe(2);
});
