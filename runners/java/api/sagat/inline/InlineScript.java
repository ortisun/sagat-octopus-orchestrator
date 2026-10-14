package sagat.inline;

/** The stable sagat-inline-v1 entry contract. */
public interface InlineScript {
    Object run(Object input, Octopus octopus) throws Exception;
}
