import com.sun.source.tree.BlockTree;
import com.sun.source.tree.BreakTree;
import com.sun.source.tree.CaseTree;
import com.sun.source.tree.ClassTree;
import com.sun.source.tree.ContinueTree;
import com.sun.source.tree.IdentifierTree;
import com.sun.source.tree.LiteralTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.MethodInvocationTree;
import com.sun.source.tree.MethodTree;
import com.sun.source.tree.NewArrayTree;
import com.sun.source.tree.NewClassTree;
import com.sun.source.tree.Tree;
import com.sun.source.tree.TypeCastTree;
import com.sun.source.tree.VariableTree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.TreeScanner;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collections;
import java.util.List;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.ToolProvider;

/** Emits complete, formatting-independent javac ASTs for the port audit. */
public final class JavaAstAuditDump {
    private JavaAstAuditDump() {}

    private static String key(MethodTree method) {
        StringBuilder result = new StringBuilder();
        result.append(method.getName()).append('(');
        for (int index = 0; index < method.getParameters().size(); index++) {
            if (index != 0) result.append(',');
            result.append(method.getParameters().get(index).getType());
        }
        return result.append(')').toString();
    }

    private static String encoded(String value) {
        return Base64.getEncoder().encodeToString(value.getBytes(StandardCharsets.UTF_8));
    }

    private static void emit(
            String source, String owner, String key, String ast, List<String> bodyNodes) {
        System.out.println(source + "\t" + owner + "\t" + key + "\t"
                + encoded(ast) + "\t" + encoded(String.join("\n", bodyNodes)));
    }

    /** Stable pre-order inventory of every javac node in one executable body. */
    private static final class BodyNodeScanner extends TreeScanner<Void, Void> {
        final List<String> nodes = new ArrayList<>();

        private String detail(Tree tree) {
            if (tree instanceof LiteralTree literal) {
                return String.valueOf(literal.getValue());
            }
            if (tree instanceof IdentifierTree identifier) {
                return identifier.getName().toString();
            }
            if (tree instanceof MemberSelectTree member) {
                return member.getIdentifier().toString();
            }
            if (tree instanceof VariableTree variable) {
                return variable.getName() + "\t" + variable.getType();
            }
            if (tree instanceof MethodInvocationTree call) {
                return call.getMethodSelect() + "\t" + call.getArguments().size();
            }
            if (tree instanceof NewClassTree allocation) {
                return allocation.getIdentifier() + "\t" + allocation.getArguments().size();
            }
            if (tree instanceof NewArrayTree allocation) {
                int initializers = allocation.getInitializers() == null
                        ? -1 : allocation.getInitializers().size();
                return allocation.getType() + "\t" + allocation.getDimensions().size()
                        + "\t" + initializers;
            }
            if (tree instanceof TypeCastTree cast) {
                return cast.getType().toString();
            }
            if (tree instanceof BreakTree statement) {
                return String.valueOf(statement.getLabel());
            }
            if (tree instanceof ContinueTree statement) {
                return String.valueOf(statement.getLabel());
            }
            if (tree instanceof CaseTree statement) {
                return statement.getCaseKind().toString();
            }
            return "";
        }

        @Override
        public Void scan(Tree tree, Void unused) {
            if (tree != null) nodes.add(tree.getKind() + "\t" + detail(tree));
            return super.scan(tree, unused);
        }
    }

    private static List<String> bodyNodes(List<? extends Tree> trees) {
        BodyNodeScanner scanner = new BodyNodeScanner();
        for (Tree tree : trees) scanner.scan(tree, null);
        return scanner.nodes;
    }

    private static final class Scanner extends TreePathScanner<Void, String> {
        private String qualifiedOwner() {
            List<String> names = new ArrayList<>();
            for (TreePath path = getCurrentPath(); path != null; path = path.getParentPath()) {
                if (path.getLeaf() instanceof ClassTree owner
                        && !owner.getSimpleName().isEmpty()) {
                    names.add(owner.getSimpleName().toString());
                }
            }
            Collections.reverse(names);
            return String.join(".", names);
        }

        @Override
        public Void visitClass(ClassTree tree, String source) {
            List<String> instanceInitializers = new ArrayList<>();
            List<String> staticInitializers = new ArrayList<>();
            List<Tree> instanceInitializerTrees = new ArrayList<>();
            List<Tree> staticInitializerTrees = new ArrayList<>();
            List<MethodTree> methods = new ArrayList<>();
            List<ClassTree> nestedClasses = new ArrayList<>();
            boolean hasConstructor = false;
            String owner = qualifiedOwner();

            for (Tree member : tree.getMembers()) {
                if (member instanceof VariableTree variable) {
                    emit(source, owner, "<field:" + variable.getName() + ">",
                            variable.toString(), bodyNodes(List.of(variable)));
                    if (variable.getInitializer() != null) {
                        String item = variable.getName() + "=" + variable.getInitializer();
                        if (variable.getModifiers().getFlags().contains(
                                javax.lang.model.element.Modifier.STATIC)) {
                            staticInitializers.add(item);
                            staticInitializerTrees.add(variable.getInitializer());
                        } else {
                            instanceInitializers.add(item);
                            instanceInitializerTrees.add(variable.getInitializer());
                        }
                    }
                } else if (member instanceof BlockTree block && block.isStatic()) {
                    staticInitializers.add(block.toString());
                    staticInitializerTrees.add(block);
                } else if (member instanceof MethodTree method) {
                    methods.add(method);
                    hasConstructor |= method.getName().contentEquals("<init>");
                } else if (member instanceof ClassTree nestedClass) {
                    nestedClasses.add(nestedClass);
                }
            }

            if (!hasConstructor) {
                List<String> nodes = new ArrayList<>();
                nodes.add("SYNTHETIC_SUPER_CONSTRUCTOR\t");
                nodes.addAll(bodyNodes(instanceInitializerTrees));
                emit(source, owner, "<init>()",
                        "implicit-super();instance-initializers:" + instanceInitializers,
                        nodes);
            }
            for (MethodTree method : methods) {
                String ast = method.toString();
                List<Tree> executableTrees = new ArrayList<>();
                boolean constructor = method.getName().contentEquals("<init>");
                boolean delegatesThis = constructor
                        && ast.matches("(?s).*\\{\\s*this\\s*\\(.*");
                boolean explicitSuper = constructor
                        && ast.matches("(?s).*\\{\\s*super\\s*\\(.*");
                List<String> nodes = new ArrayList<>();
                if (constructor && !delegatesThis) {
                    if (!explicitSuper) nodes.add("SYNTHETIC_SUPER_CONSTRUCTOR\t");
                    String prologue = explicitSuper ? "explicit-super();" : "implicit-super();";
                    ast = prologue + "instance-initializers:" + instanceInitializers + ";" + ast;
                    nodes.addAll(bodyNodes(instanceInitializerTrees));
                }
                if (method.getBody() != null) executableTrees.add(method.getBody());
                nodes.addAll(bodyNodes(executableTrees));
                emit(source, owner, key(method), ast, nodes);
            }
            emit(source, owner, "<clinit>()", staticInitializers.toString(),
                    bodyNodes(staticInitializerTrees));
            for (ClassTree nestedClass : nestedClasses) scan(nestedClass, source);
            return null;
        }
    }

    public static void main(String[] args) throws Exception {
        if (args.length == 0) {
            System.err.println("usage: JavaAstAuditDump JAVA_SOURCE ...");
            System.exit(2);
        }
        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        var manager = compiler.getStandardFileManager(null, null, StandardCharsets.UTF_8);
        List<File> files = new ArrayList<>();
        for (String arg : args) files.add(new File(arg));
        Iterable<? extends JavaFileObject> units = manager.getJavaFileObjectsFromFiles(files);
        JavacTask task = (JavacTask) compiler.getTask(
                null, manager, null, List.of("-proc:none"), null, units);
        for (var unit : task.parse()) {
            new Scanner().scan(unit, unit.getSourceFile().toUri().getPath());
        }
        manager.close();
    }
}
