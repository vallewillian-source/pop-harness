import { structure as ptBR_structure } from "../pt-BR/structure";

export const structure: Record<keyof typeof ptBR_structure, string> = {
    statusActive: "Active",
    statusInactive: "Inactive",
    statusArchived: "Archived",
    validationCategoryNameRequired: "enter the category name",
    validationDepartmentNameRequired: "enter the department name",
    validationNameMaxLength: "the name accepts up to 120 characters",
    conflictCategory:
        "Someone else saved this category before you{version}. What you typed was kept: review and save again.",
    conflictDepartment:
        "Someone else saved this department before you{version}. What you typed was kept: review and save again.",
    conflictVersionSuffix: " (now at version {version})",
    unexpectedSaveCategoryError: "unexpected failure while saving the category",
    unexpectedSaveDepartmentError: "unexpected failure while saving the department",
    modalTitleEditCategory: "Edit category",
    modalTitleEditDepartment: "Edit department",
    modalTitleNewCategory: "New category",
    modalTitleNewDepartment: "New department",
    modalDescriptionEdit: "Editing {name}. Saving uses the version that was read.",
    modalDescriptionNewCategory:
        "Groups catalog products and filters what each agent can offer.",
    modalDescriptionNewDepartment:
        "Handoff destination for human support and default for new conversations.",
    modalCloseCategory: "Close category form",
    modalCloseDepartment: "Close department form",
    cancel: "Cancel",
    saveChanges: "Save changes",
    createCategory: "Create category",
    createDepartment: "Create department",
    bannerOutdatedTitle: "Outdated version",
    bannerSaveErrorTitle: "Could not save",
    fieldNameLabel: "Name",
    fieldNameHint:
        "Unique in the organization, case-insensitive. Up to 120 characters.",
    fieldDescriptionLabel: "Description",
    fieldDescriptionHint: "Optional. Shown in the list and in the operator conversation.",
    fieldStatusLabel: "Status",
    fieldStatusHint:
        "Inactive and archived no longer apply to agents; the record remains in the database.",
    deleteDeptTitle: "Delete department?",
    deleteDeptDescription: "Department {name} will be removed from the system. This cannot be undone.",
    deleteDeptResourceLabel: "department name",
    deleteDeptConfirm: "Delete department",
    deleteDeptCheckFailed:
        "Could not check which agents use this department. Agents that use it, as default or for handoff, will lose it after deletion.",
    deleteDeptNoAgents:
        "No agent uses this department today. Only the record is removed from the system.",
    deleteDeptAgentsOnePrefix: "1 agent uses this department",
    deleteDeptAgentsManyPrefix: "{count} agents use this department",
    deleteDeptAgentsMiddle: "(default or handoff) and will lose it:",
    deleteDeptAgentsSuffix:
        "Afterward, these agents will need another department to transfer or receive new conversations.",
    deleteCatTitle: "Delete category?",
    deleteCatDescription: "Category {name} will be removed from the system. This cannot be undone.",
    deleteCatResourceLabel: "category name",
    deleteCatConfirm: "Delete category",
    deleteCatCheckFailed:
        "Could not count the products and agents in this category. If any exist, the API will refuse deletion and show the count.",
    deleteCatNoLinks:
        "No product or agent uses this category. Only the record is removed from the system.",
    deleteCatProductOne: "1 product",
    deleteCatProductMany: "{count} products",
    deleteCatProductsLine:
        "This category has {products} (of any status). Deleting removes {pronoun}:",
    deleteCatProductsPronounOne: "it",
    deleteCatProductsPronounMany: "all of them",
    deleteCatAgentOne: "1 agent uses",
    deleteCatAgentMany: "{count} agents use",
    deleteCatAgentsLine: "{agents} this category and will lose it:",
    deleteCatForceWithProductsOne: "Also delete the 1 product in this category",
    deleteCatForceWithProductsMany: "Also delete the {count} products in this category",
    deleteCatForceAgentsOnly: "Remove the category from agents and delete",
    deleteCatForceDescription:
        "Without this confirmation, the API refuses deletion while the category has linked products or agents.",
    agentsCountUnknown: "agents: —",
    agentsCountOne: "1 agent",
    agentsCountMany: "{count} agents",
    departmentsTitle: "Departments",
    categoriesTitle: "Product categories",
    loadDepartmentsError: "failed to load departments",
    loadCategoriesError: "failed to load product categories",
    loadingDepartments: "Loading departments",
    loadingCategories: "Loading product categories",
    errorDepartmentsTitle: "Could not load departments",
    errorCategoriesTitle: "Could not load product categories",
    errorDescription:
        "The instance did not respond. Other lists on this screen continue working.",
    retry: "Try again",
    emptyDepartmentsTitle: "No departments registered",
    emptyCategoriesTitle: "No categories registered",
    emptyDepartmentsDescription:
        "Without a department, the agent has nowhere to transfer the conversation.",
    emptyCategoriesDescription:
        "Categories group products and control what each agent can offer.",
    createFirstDepartment: "Create first department",
    createFirstCategory: "Create first category",
    listDepartmentsAria: "List of departments",
    listCategoriesAria: "List of product categories",
    noDescription: "No description",
    edit: "Edit",
    delete: "Delete",
    editDepartmentAria: "Edit department {name}",
    editCategoryAria: "Edit category {name}",
    deleteDepartmentAria: "Delete department {name}",
    deleteCategoryAria: "Delete category {name}",
    departmentsHelp:
        "Handoff destinations for human support and default for new conversations.",
    categoriesHelp:
        "Group catalog products and filter what each agent can offer.",
    refresh: "Refresh",
    eyebrowOrg: "Organization · {name}",
    eyebrow: "Organization",
    pageTitle: "Departments and categories",
    pageDescription:
        "Structure agents use to transfer conversations and offer the catalog.",
    refreshAgents: "Refresh agents",
};
