<script lang="ts" module>
  import arrowPath from 'heroicons/24/outline/arrow-path.svg?raw';
  import arrowRightStartOnRectangle from 'heroicons/24/outline/arrow-right-start-on-rectangle.svg?raw';
  import arrowTopRightOnSquare from 'heroicons/24/outline/arrow-top-right-on-square.svg?raw';
  import check from 'heroicons/24/outline/check.svg?raw';
  import checkCircle from 'heroicons/24/outline/check-circle.svg?raw';
  import clipboardDocument from 'heroicons/24/outline/clipboard-document.svg?raw';
  import exclamationTriangle from 'heroicons/24/outline/exclamation-triangle.svg?raw';
  import informationCircle from 'heroicons/24/outline/information-circle.svg?raw';
  import plus from 'heroicons/24/outline/plus.svg?raw';
  import trash from 'heroicons/24/outline/trash.svg?raw';

  const ICONS = {
    'arrow-path': arrowPath,
    'arrow-right-start-on-rectangle': arrowRightStartOnRectangle,
    'arrow-top-right-on-square': arrowTopRightOnSquare,
    check,
    'check-circle': checkCircle,
    'clipboard-document': clipboardDocument,
    'exclamation-triangle': exclamationTriangle,
    'information-circle': informationCircle,
    plus,
    trash,
  } as const;

  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  let { name }: { name: IconName } = $props();
</script>

<!-- The heroicons markup is static and marks itself aria-hidden: the text next to it names
     what it shows. -->
<!-- eslint-disable-next-line svelte/no-at-html-tags -->
<span class="icon">{@html ICONS[name]}</span>

<style>
  .icon {
    display: inline-flex;
  }

  .icon :global(svg) {
    width: 100%;
    height: 100%;
  }
</style>
