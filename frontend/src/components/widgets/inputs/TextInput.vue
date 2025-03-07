<template>
	<FieldInput
		class="text-input"
		v-model:value="text"
		:label="label"
		:spellcheck="true"
		:disabled="disabled"
		@textFocused="() => onTextFocused()"
		@textChanged="() => onTextChanged()"
		@cancelTextChange="() => onCancelTextChange()"
		ref="fieldInput"
	></FieldInput>
</template>

<style lang="scss"></style>

<script lang="ts">
import { defineComponent, PropType } from "vue";

import FieldInput from "@/components/widgets/inputs/FieldInput.vue";

export default defineComponent({
	emits: ["update:value"],
	props: {
		value: { type: String as PropType<string>, required: true },
		label: { type: String as PropType<string>, required: false },
		disabled: { type: Boolean as PropType<boolean>, default: false },
	},
	data() {
		return {
			editing: false,
		};
	},
	computed: {
		text: {
			get() {
				return this.value;
			},
			set(value: string) {
				this.$emit("update:value", value);
			},
		},
	},
	methods: {
		onTextFocused() {
			this.editing = true;

			const inputElement = (this.$refs.fieldInput as typeof FieldInput).$refs.input as HTMLInputElement;
			// Setting the value directly is required to make `inputElement.select()` work
			inputElement.value = this.text;
			inputElement.select();
		},
		// Called only when `value` is changed from the <input> element via user input and committed, either with the
		// enter key (via the `change` event) or when the <input> element is defocused (with the `blur` event binding)
		onTextChanged() {
			// The `inputElement.blur()` call in `onCancelTextChange()` causes itself to be run again, so this if statement skips a second run
			if (this.editing) this.onCancelTextChange();
		},
		onCancelTextChange() {
			this.editing = false;

			const inputElement = (this.$refs.fieldInput as typeof FieldInput).$refs.input as HTMLInputElement;
			inputElement.blur();
		},
	},
	components: { FieldInput },
});
</script>
