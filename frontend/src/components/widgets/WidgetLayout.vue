<template>
	<div class="widget-layout">
		<template v-for="(layoutRow, index) in layout.layout" :key="index">
			<component :is="layoutRowType(layoutRow)" :widgetData="layoutRow" :layoutTarget="layout.layout_target"></component>
		</template>
	</div>
</template>

<style lang="scss">
.widget-layout {
	height: 100%;
	flex: 0 0 auto;
	display: flex;
	flex-direction: column;
	align-items: center;
}
</style>

<script lang="ts">
import { defineComponent, PropType } from "vue";

import { isWidgetRow, isWidgetSection, LayoutRow, WidgetLayout } from "@/dispatcher/js-messages";

import WidgetRow from "@/components/widgets/WidgetRow.vue";
import WidgetSection from "@/components/widgets/WidgetSection.vue";

export default defineComponent({
	props: {
		layout: { type: Object as PropType<WidgetLayout>, required: true },
	},
	methods: {
		layoutRowType(layoutRow: LayoutRow): unknown {
			if (isWidgetRow(layoutRow)) return WidgetRow;
			if (isWidgetSection(layoutRow)) return WidgetSection;

			throw new Error("Layout row type does not exist");
		},
	},
	data: () => {
		return {
			isWidgetRow,
			isWidgetSection,
		};
	},
	components: {
		WidgetRow,
		WidgetSection,
	},
});
</script>
