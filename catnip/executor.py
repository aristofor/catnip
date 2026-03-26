# FILE: catnip/executor.py
import logging


class Executor:
    """
    Main executor for Catnip code.

    Executes code by interpreting AST nodes directly via the registry.
    """

    def __init__(self, registry, context, logger=None):
        self.registry = registry
        self.context = context
        self.logger = logger or context.logger or logging.getLogger(__name__)
        self.execute_statement = self.registry.exec_stmt

    def execute(self, statements, trace=False):
        """
        Execute statements by interpreting AST directly.

        :param statements: List of Op nodes to execute
        :param trace: Enable execution tracing
        :return: Result of the last statement
        """
        self.context.result = None
        self.current_stmt = None
        for stmt in statements:
            self.current_stmt = stmt
            if trace:
                self.logger.debug(f"Executing: {stmt}")
            self.context.result = self.execute_statement(stmt)
        return self.context.result
