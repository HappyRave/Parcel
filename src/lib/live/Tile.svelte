<script lang="ts">
  import { paint } from "$lib/api";

  /** A letter on the app's or service's colour, standing in for its icon. */
  let { label, color, large = false }: { label: string; color: number | null; large?: boolean } =
    $props();

  const letter = $derived((label.match(/[\p{L}\p{N}]/u)?.[0] ?? "?").toUpperCase());
</script>

<span class="tile" class:large style:background={paint(color)} aria-hidden="true">{letter}</span>

<style>
  .tile {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    display: inline-grid;
    place-items: center;
    font: 700 10px var(--ui);
    color: #fff;
    flex: none;
  }
  .large {
    width: 22px;
    height: 22px;
    font-size: 12px;
    border-radius: 5px;
  }
</style>
